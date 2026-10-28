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
  pub(crate) fn of_render_process(did_crash: bool) -> Self {
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

pub(crate) fn make_again<M: MadeWith>(
  made_with: M,
  handler: Option<PageLostHandler>,
  lost: PageLost,
  let_go_of_lost: impl FnOnce(),
  create: impl FnOnce(M),
) {
  let_go_of_lost();
  match handler {
    Some(handler) => {
      create(made_with.loading_nothing());
      handler(lost);
    }
    None => create(made_with),
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use std::{cell::RefCell, rc::Rc};

  const EVERY_WAY_A_PAGE_IS_LOST: [PageLost; 3] = [
    PageLost::RenderProcessCrashed,
    PageLost::RenderProcessKilled,
    PageLost::ActivityRecreated,
  ];

  const ADDRESS: &str = "https://dioxus.index.html/";

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

  #[derive(Debug, PartialEq, Eq)]
  enum Step {
    LetGoOfLost,
    Created(Made),
    Told(PageLost),
  }

  fn made_with(url: Option<&'static str>, html: Option<&'static str>) -> Made {
    Made {
      id: "7",
      url,
      html,
      transparent: true,
    }
  }

  fn steps_of(made: Made, taken_over: bool, lost: PageLost) -> Vec<Step> {
    let steps = Rc::new(RefCell::new(Vec::new()));
    let handler = taken_over.then(|| {
      let steps = steps.clone();
      Arc::new(move |lost| steps.borrow_mut().push(Step::Told(lost))) as PageLostHandler
    });
    make_again(
      made,
      handler,
      lost,
      || steps.borrow_mut().push(Step::LetGoOfLost),
      |made| steps.borrow_mut().push(Step::Created(made)),
    );
    steps.take()
  }

  #[test]
  fn a_page_the_application_takes_over_is_made_again_blank_before_the_application_is_told() {
    for lost in EVERY_WAY_A_PAGE_IS_LOST {
      assert_eq!(
        steps_of(made_with(Some(ADDRESS), None), true, lost),
        vec![
          Step::LetGoOfLost,
          Step::Created(made_with(None, None)),
          Step::Told(lost),
        ],
        "The replacement loads nothing, so the page the application asks for once told is the \
         only load ({lost:?})."
      );
    }
  }

  #[test]
  fn a_page_made_from_html_and_taken_over_is_made_again_blank_too() {
    for lost in EVERY_WAY_A_PAGE_IS_LOST {
      assert_eq!(
        steps_of(made_with(None, Some("<p></p>")), true, lost),
        vec![
          Step::LetGoOfLost,
          Step::Created(made_with(None, None)),
          Step::Told(lost),
        ],
        "{lost:?}"
      );
    }
  }

  #[test]
  fn a_page_no_application_takes_over_is_made_again_loading_what_it_was_made_to_load() {
    for lost in EVERY_WAY_A_PAGE_IS_LOST {
      assert_eq!(
        steps_of(made_with(Some(ADDRESS), None), false, lost),
        vec![
          Step::LetGoOfLost,
          Step::Created(made_with(Some(ADDRESS), None)),
        ],
        "{lost:?}"
      );
      assert_eq!(
        steps_of(made_with(None, Some("<p></p>")), false, lost),
        vec![
          Step::LetGoOfLost,
          Step::Created(made_with(None, Some("<p></p>"))),
        ],
        "{lost:?}"
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
    assert_eq!(
      steps_of(made.clone(), true, PageLost::ActivityRecreated),
      vec![
        Step::LetGoOfLost,
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
  fn the_application_is_told_once_for_each_page_lost() {
    let told = Rc::new(RefCell::new(Vec::new()));
    let handler: PageLostHandler = {
      let told = told.clone();
      Arc::new(move |lost| told.borrow_mut().push(lost))
    };
    for lost in EVERY_WAY_A_PAGE_IS_LOST {
      make_again(
        made_with(Some(ADDRESS), None),
        Some(handler.clone()),
        lost,
        || {},
        |_| {},
      );
    }
    assert_eq!(*told.borrow(), EVERY_WAY_A_PAGE_IS_LOST);
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
