// Copyright 2020-2023 Tauri Programme within The Commons Conservancy
// SPDX-License-Identifier: Apache-2.0
// SPDX-License-Identifier: MIT

use std::sync::Arc;

/// How the page a web view held was lost, as Android reports it to
/// [`WebViewBuilderExtAndroid::with_on_page_lost_handler`](crate::WebViewBuilderExtAndroid::with_on_page_lost_handler).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PageLost {
  /// The web view's renderer process crashed.
  RenderProcessCrashed,
  /// The system ended the web view's renderer process, most often to reclaim its memory.
  RenderProcessKilled,
  /// The activity the web view was in was destroyed and created again in the same process, as
  /// Android does to show a configuration change the activity does not handle itself, and the web
  /// view went with it.
  ActivityRecreated,
}

impl PageLost {
  fn of_render_process(did_crash: bool) -> Self {
    if did_crash {
      Self::RenderProcessCrashed
    } else {
      Self::RenderProcessKilled
    }
  }
}

pub(crate) type PageLostHandler = Arc<dyn Fn(PageLost)>;

pub(crate) trait MadeWith {
  fn loading_nothing(self) -> Self;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Report<V> {
  RenderProcessGone { webview: V, did_crash: bool },
  ActivityRecreated,
}

impl<V> Report<V> {
  fn lost(&self) -> PageLost {
    match self {
      Self::RenderProcessGone { did_crash, .. } => PageLost::of_render_process(*did_crash),
      Self::ActivityRecreated => PageLost::ActivityRecreated,
    }
  }
}

pub(crate) trait Activity {
  type WebView;
  type Attributes: MadeWith;
  type Error;

  fn webview(&self) -> Option<Self::WebView>;
  fn is_same(&self, one: &Self::WebView, other: &Self::WebView) -> Result<bool, Self::Error>;
  fn forget_webview(&mut self);
  fn destroy(&mut self, webview: Self::WebView) -> Result<(), Self::Error>;
  fn create(&mut self, made_with: Self::Attributes);
}

pub(crate) fn make_again<A: Activity>(
  activity: &mut A,
  made_with: A::Attributes,
  handler: Option<PageLostHandler>,
  report: Report<A::WebView>,
) -> Result<(), A::Error> {
  let Some(held) = activity.webview() else {
    return Ok(());
  };
  let lost = report.lost();
  match report {
    Report::RenderProcessGone { webview, .. } => {
      if !activity.is_same(&webview, &held)? {
        return Ok(());
      }
      activity.forget_webview();
    }
    Report::ActivityRecreated => {
      activity.forget_webview();
      activity.destroy(held)?;
    }
  }
  match handler {
    Some(handler) => {
      activity.create(made_with.loading_nothing());
      handler(lost);
    }
    None => activity.create(made_with),
  }
  Ok(())
}

#[cfg(test)]
mod tests {
  use super::*;
  use std::sync::Mutex;

  const ADDRESS: &str = "https://dioxus.index.html/";

  const REPLACED: View = View(0);
  const HELD: View = View(1);
  const REPLACEMENT: View = View(2);

  const RENDER_PROCESS_GONE: [(bool, PageLost); 2] = [
    (true, PageLost::RenderProcessCrashed),
    (false, PageLost::RenderProcessKilled),
  ];

  #[derive(Debug, Clone, Copy, PartialEq, Eq)]
  struct View(u8);

  #[derive(Debug, Clone, PartialEq, Eq)]
  struct Made {
    id: &'static str,
    url: Option<&'static str>,
    html: Option<&'static str>,
    transparent: bool,
  }

  impl MadeWith for Made {
    fn loading_nothing(self) -> Self {
      Self {
        url: None,
        html: None,
        ..self
      }
    }
  }

  #[derive(Debug, Clone, Copy, PartialEq, Eq)]
  enum Fault {
    Comparing,
    Destroying,
  }

  #[derive(Debug, PartialEq, Eq)]
  enum Step {
    Forgot,
    Destroyed(View),
    Created(Made),
    Told(PageLost),
  }

  struct Held {
    webview: Option<View>,
    fault: Option<Fault>,
    steps: Arc<Mutex<Vec<Step>>>,
  }

  impl Activity for Held {
    type WebView = View;
    type Attributes = Made;
    type Error = Fault;

    fn webview(&self) -> Option<View> {
      self.webview
    }

    fn is_same(&self, one: &View, other: &View) -> Result<bool, Fault> {
      match self.fault {
        Some(Fault::Comparing) => Err(Fault::Comparing),
        Some(Fault::Destroying) | None => Ok(one == other),
      }
    }

    fn forget_webview(&mut self) {
      self.webview = None;
      self.steps.lock().unwrap().push(Step::Forgot);
    }

    fn destroy(&mut self, webview: View) -> Result<(), Fault> {
      match self.fault {
        Some(Fault::Destroying) => Err(Fault::Destroying),
        Some(Fault::Comparing) | None => {
          self.steps.lock().unwrap().push(Step::Destroyed(webview));
          Ok(())
        }
      }
    }

    fn create(&mut self, made_with: Made) {
      self.steps.lock().unwrap().push(Step::Created(made_with));
    }
  }

  struct Scene {
    activity: Held,
    handler: Option<PageLostHandler>,
  }

  impl Scene {
    fn holding(webview: Option<View>, taken_over: bool) -> Self {
      let steps = Arc::new(Mutex::new(Vec::new()));
      let handler = taken_over.then(|| {
        let steps = steps.clone();
        Arc::new(move |lost| steps.lock().unwrap().push(Step::Told(lost))) as PageLostHandler
      });
      Self {
        activity: Held {
          webview,
          fault: None,
          steps,
        },
        handler,
      }
    }

    fn failing_at(mut self, fault: Fault) -> Self {
      self.activity.fault = Some(fault);
      self
    }

    fn report(&mut self, made: Made, report: Report<View>) -> Result<(), Fault> {
      make_again(&mut self.activity, made, self.handler.clone(), report)
    }

    fn make_replacement(&mut self, webview: View) {
      self.activity.webview = Some(webview);
    }

    fn webview(&self) -> Option<View> {
      self.activity.webview
    }

    fn steps(&self) -> Vec<Step> {
      std::mem::take(&mut *self.activity.steps.lock().unwrap())
    }
  }

  fn made_with(url: Option<&'static str>, html: Option<&'static str>) -> Made {
    Made {
      id: "7",
      url,
      html,
      transparent: true,
    }
  }

  fn gone(webview: View, did_crash: bool) -> Report<View> {
    Report::RenderProcessGone { webview, did_crash }
  }

  #[test]
  fn a_view_whose_renderer_went_is_made_again_blank_before_the_application_is_told() {
    for (did_crash, lost) in RENDER_PROCESS_GONE {
      let mut scene = Scene::holding(Some(HELD), true);
      assert_eq!(
        scene.report(made_with(Some(ADDRESS), None), gone(HELD, did_crash)),
        Ok(())
      );
      assert_eq!(
        scene.steps(),
        vec![
          Step::Forgot,
          Step::Created(made_with(None, None)),
          Step::Told(lost),
        ],
        "The view its client already destroyed is let go without being destroyed again, and the \
         replacement loads nothing, so the page the application asks for once told is the only \
         load ({lost:?})."
      );
      assert_eq!(scene.webview(), None);
    }
  }

  #[test]
  fn the_view_of_an_activity_created_again_is_destroyed_before_its_replacement_is_asked_for() {
    let mut scene = Scene::holding(Some(HELD), true);
    assert_eq!(
      scene.report(made_with(Some(ADDRESS), None), Report::ActivityRecreated),
      Ok(())
    );
    assert_eq!(
      scene.steps(),
      vec![
        Step::Forgot,
        Step::Destroyed(HELD),
        Step::Created(made_with(None, None)),
        Step::Told(PageLost::ActivityRecreated),
      ]
    );
    assert_eq!(scene.webview(), None);
  }

  #[test]
  fn the_view_of_an_activity_created_again_is_destroyed_even_when_no_application_takes_over() {
    let mut scene = Scene::holding(Some(HELD), false);
    assert_eq!(
      scene.report(made_with(Some(ADDRESS), None), Report::ActivityRecreated),
      Ok(())
    );
    assert_eq!(
      scene.steps(),
      vec![
        Step::Forgot,
        Step::Destroyed(HELD),
        Step::Created(made_with(Some(ADDRESS), None)),
      ]
    );
    assert_eq!(scene.webview(), None);
  }

  #[test]
  fn a_page_made_from_html_and_taken_over_is_made_again_blank_too() {
    for (did_crash, lost) in RENDER_PROCESS_GONE {
      let mut scene = Scene::holding(Some(HELD), true);
      assert_eq!(
        scene.report(made_with(None, Some("<p></p>")), gone(HELD, did_crash)),
        Ok(())
      );
      assert_eq!(
        scene.steps(),
        vec![
          Step::Forgot,
          Step::Created(made_with(None, None)),
          Step::Told(lost),
        ],
        "{lost:?}"
      );
    }
    let mut scene = Scene::holding(Some(HELD), true);
    assert_eq!(
      scene.report(made_with(None, Some("<p></p>")), Report::ActivityRecreated),
      Ok(())
    );
    assert_eq!(
      scene.steps(),
      vec![
        Step::Forgot,
        Step::Destroyed(HELD),
        Step::Created(made_with(None, None)),
        Step::Told(PageLost::ActivityRecreated),
      ]
    );
  }

  #[test]
  fn a_page_no_application_takes_over_is_made_again_loading_what_it_was_made_to_load() {
    for made in [
      made_with(Some(ADDRESS), None),
      made_with(None, Some("<p></p>")),
    ] {
      for (did_crash, lost) in RENDER_PROCESS_GONE {
        let mut scene = Scene::holding(Some(HELD), false);
        assert_eq!(scene.report(made.clone(), gone(HELD, did_crash)), Ok(()));
        assert_eq!(
          scene.steps(),
          vec![Step::Forgot, Step::Created(made.clone())],
          "{lost:?}"
        );
      }
      let mut scene = Scene::holding(Some(HELD), false);
      assert_eq!(
        scene.report(made.clone(), Report::ActivityRecreated),
        Ok(())
      );
      assert_eq!(
        scene.steps(),
        vec![
          Step::Forgot,
          Step::Destroyed(HELD),
          Step::Created(made.clone()),
        ]
      );
    }
  }

  #[test]
  fn the_replacement_keeps_every_attribute_but_what_it_loads() {
    let made = Made {
      id: "42",
      url: Some(ADDRESS),
      html: None,
      transparent: false,
    };
    let mut scene = Scene::holding(Some(HELD), true);
    assert_eq!(scene.report(made, Report::ActivityRecreated), Ok(()));
    assert_eq!(
      scene.steps(),
      vec![
        Step::Forgot,
        Step::Destroyed(HELD),
        Step::Created(Made {
          id: "42",
          url: None,
          html: None,
          transparent: false,
        }),
        Step::Told(PageLost::ActivityRecreated),
      ]
    );
  }

  #[test]
  fn a_report_from_a_view_the_activity_no_longer_holds_changes_nothing() {
    for taken_over in [true, false] {
      for (did_crash, lost) in RENDER_PROCESS_GONE {
        let mut scene = Scene::holding(Some(HELD), taken_over);
        assert_eq!(
          scene.report(made_with(Some(ADDRESS), None), gone(REPLACED, did_crash)),
          Ok(())
        );
        assert_eq!(
          scene.steps(),
          vec![],
          "A view already replaced neither replaces the view held nor tells the application \
           ({lost:?}, taken over: {taken_over})."
        );
        assert_eq!(scene.webview(), Some(HELD));
      }
    }
  }

  #[test]
  fn an_activity_holding_no_view_ignores_every_report() {
    for taken_over in [true, false] {
      for report in [
        gone(HELD, true),
        gone(HELD, false),
        Report::ActivityRecreated,
      ] {
        let mut scene = Scene::holding(None, taken_over);
        assert_eq!(scene.report(made_with(Some(ADDRESS), None), report), Ok(()));
        assert_eq!(
          scene.steps(),
          vec![],
          "The replacement already asked for is the one the activity takes ({report:?}, taken \
           over: {taken_over})."
        );
        assert_eq!(scene.webview(), None);
      }
    }
  }

  #[test]
  fn a_renderer_that_took_an_old_view_and_the_current_one_with_it_makes_one_replacement() {
    for (did_crash, lost) in RENDER_PROCESS_GONE {
      let mut scene = Scene::holding(Some(HELD), true);
      assert_eq!(
        scene.report(made_with(Some(ADDRESS), None), gone(REPLACED, did_crash)),
        Ok(())
      );
      assert_eq!(
        scene.report(made_with(Some(ADDRESS), None), gone(HELD, did_crash)),
        Ok(())
      );
      assert_eq!(
        scene.steps(),
        vec![
          Step::Forgot,
          Step::Created(made_with(None, None)),
          Step::Told(lost),
        ],
        "The old view reports first ({lost:?})."
      );

      let mut scene = Scene::holding(Some(HELD), true);
      assert_eq!(
        scene.report(made_with(Some(ADDRESS), None), gone(HELD, did_crash)),
        Ok(())
      );
      assert_eq!(
        scene.report(made_with(Some(ADDRESS), None), gone(REPLACED, did_crash)),
        Ok(())
      );
      assert_eq!(
        scene.steps(),
        vec![
          Step::Forgot,
          Step::Created(made_with(None, None)),
          Step::Told(lost),
        ],
        "The old view reports while the replacement is still being made ({lost:?})."
      );

      let mut scene = Scene::holding(Some(HELD), true);
      assert_eq!(
        scene.report(made_with(Some(ADDRESS), None), gone(HELD, did_crash)),
        Ok(())
      );
      scene.make_replacement(REPLACEMENT);
      assert_eq!(
        scene.report(made_with(Some(ADDRESS), None), gone(REPLACED, did_crash)),
        Ok(())
      );
      assert_eq!(
        scene.steps(),
        vec![
          Step::Forgot,
          Step::Created(made_with(None, None)),
          Step::Told(lost),
        ],
        "The old view reports once the replacement is made ({lost:?})."
      );
      assert_eq!(scene.webview(), Some(REPLACEMENT));
    }
  }

  #[test]
  fn the_application_is_told_once_for_each_page_lost() {
    let mut scene = Scene::holding(Some(View(10)), true);
    let reports = [
      Report::ActivityRecreated,
      gone(View(11), true),
      gone(View(12), false),
      Report::ActivityRecreated,
    ];
    for (made, report) in (11..).map(View).zip(reports) {
      assert_eq!(scene.report(made_with(Some(ADDRESS), None), report), Ok(()));
      scene.make_replacement(made);
    }
    assert_eq!(
      scene
        .steps()
        .into_iter()
        .filter_map(|step| match step {
          Step::Told(lost) => Some(lost),
          Step::Forgot | Step::Destroyed(_) | Step::Created(_) => None,
        })
        .collect::<Vec<_>>(),
      vec![
        PageLost::ActivityRecreated,
        PageLost::RenderProcessCrashed,
        PageLost::RenderProcessKilled,
        PageLost::ActivityRecreated,
      ]
    );
  }

  #[test]
  fn a_report_whose_view_cannot_be_compared_changes_nothing() {
    for taken_over in [true, false] {
      for (did_crash, lost) in RENDER_PROCESS_GONE {
        let mut scene = Scene::holding(Some(HELD), taken_over).failing_at(Fault::Comparing);
        assert_eq!(
          scene.report(made_with(Some(ADDRESS), None), gone(HELD, did_crash)),
          Err(Fault::Comparing)
        );
        assert_eq!(scene.steps(), vec![], "{lost:?}");
        assert_eq!(scene.webview(), Some(HELD));
      }
    }
  }

  #[test]
  fn an_activity_created_again_does_not_compare_its_view_with_any_other() {
    let mut scene = Scene::holding(Some(HELD), true).failing_at(Fault::Comparing);
    assert_eq!(
      scene.report(made_with(Some(ADDRESS), None), Report::ActivityRecreated),
      Ok(())
    );
    assert_eq!(
      scene.steps(),
      vec![
        Step::Forgot,
        Step::Destroyed(HELD),
        Step::Created(made_with(None, None)),
        Step::Told(PageLost::ActivityRecreated),
      ]
    );
  }

  #[test]
  fn a_view_that_cannot_be_destroyed_is_not_replaced() {
    for taken_over in [true, false] {
      let mut scene = Scene::holding(Some(HELD), taken_over).failing_at(Fault::Destroying);
      assert_eq!(
        scene.report(made_with(Some(ADDRESS), None), Report::ActivityRecreated),
        Err(Fault::Destroying)
      );
      assert_eq!(
        scene.steps(),
        vec![Step::Forgot],
        "No replacement is asked for and no application is told (taken over: {taken_over})."
      );
      assert_eq!(scene.webview(), None);
    }
  }

  #[test]
  fn a_view_whose_renderer_went_is_never_destroyed_again() {
    for (did_crash, lost) in RENDER_PROCESS_GONE {
      let mut scene = Scene::holding(Some(HELD), true).failing_at(Fault::Destroying);
      assert_eq!(
        scene.report(made_with(Some(ADDRESS), None), gone(HELD, did_crash)),
        Ok(())
      );
      assert_eq!(
        scene.steps(),
        vec![
          Step::Forgot,
          Step::Created(made_with(None, None)),
          Step::Told(lost),
        ],
        "{lost:?}"
      );
    }
  }

  #[test]
  fn a_renderer_that_crashed_is_told_apart_from_one_the_system_ended() {
    assert_eq!(
      PageLost::of_render_process(true),
      PageLost::RenderProcessCrashed
    );
    assert_eq!(
      PageLost::of_render_process(false),
      PageLost::RenderProcessKilled
    );
  }
}
