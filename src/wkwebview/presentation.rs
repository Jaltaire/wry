use objc2_core_foundation::CGSize;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SizeChange {
  Unchanged,
  FirstSize,
  Offscreen,
  OnScreen,
}

impl SizeChange {
  pub(crate) fn between(drawn: CGSize, now: CGSize, showing: bool) -> Self {
    if drawn.width == now.width && drawn.height == now.height {
      Self::Unchanged
    } else if drawn.width <= 0.0 || drawn.height <= 0.0 || now.width <= 0.0 || now.height <= 0.0 {
      Self::FirstSize
    } else if !showing {
      Self::Offscreen
    } else {
      Self::OnScreen
    }
  }

  pub(crate) fn waits_for_its_frame(self) -> bool {
    matches!(self, Self::OnScreen)
  }
}

pub(crate) fn cover_scale(from: CGSize, to: CGSize) -> Option<f64> {
  if from.width <= 0.0 || from.height <= 0.0 || to.width <= 0.0 || to.height <= 0.0 {
    return None;
  }
  let scale = (from.width / to.width).max(from.height / to.height);
  (scale > 1.0).then_some(scale)
}

pub(crate) fn turning_scale(from: CGSize, to: CGSize, animated: bool) -> Option<f64> {
  cover_scale(from, to).filter(|_| animated)
}

#[cfg(target_os = "ios")]
pub(crate) use ios::{follow_size, set_bounds};

#[cfg(target_os = "ios")]
mod ios {
  use std::{cell::Cell, panic::AssertUnwindSafe, rc::Rc, time::Duration};

  use block2::RcBlock;
  use objc2::{class, msg_send, sel, ClassType, DeclaredClass, MainThreadMarker};
  use objc2_core_foundation::{CFRunLoop, CFString, CGAffineTransform, CGRect};
  use objc2_foundation::{NSDate, NSObjectProtocol, NSRunLoop, NSString};
  use objc2_ui_kit::UIView;

  use super::SizeChange;
  use crate::wkwebview::{ios::WKWebView::WKWebView, WryWebView};

  pub(crate) const FRAME_PATIENCE: Duration = Duration::from_millis(500);

  const MODE: &str = "WryWaitingForFrame";

  pub(crate) fn follow_size(webview: &WryWebView) {
    let ivars = webview.ivars();
    if ivars.waiting_for_frame.get() {
      return;
    }
    let drawn = ivars.drawn_size.get();
    let now = webview.bounds().size;
    let change = SizeChange::between(drawn, now, showing(webview));
    ivars.drawn_size.set(now);
    if !change.waits_for_its_frame() {
      return;
    }
    let Some(mtm) = MainThreadMarker::new() else {
      return;
    };
    ivars.waiting_for_frame.set(true);
    wait_for_next_frame(webview, mtm);
    ivars.waiting_for_frame.set(false);
  }

  pub(crate) fn set_bounds(webview: &WryWebView, bounds: CGRect) {
    let set = |bounds: CGRect| unsafe {
      let _: () = msg_send![super(webview, WKWebView::class()), setBounds: bounds];
    };
    let Some(mtm) = MainThreadMarker::new() else {
      set(bounds);
      return;
    };
    let animated = unsafe { UIView::inheritedAnimationDuration(mtm) } > 0.0;
    let scale = super::turning_scale(webview.bounds().size, bounds.size, animated)
      .filter(|_| showing(webview));
    let Some(scale) = scale else {
      set(bounds);
      return;
    };
    unsafe {
      let enabled = UIView::areAnimationsEnabled(mtm);
      UIView::setAnimationsEnabled(false, mtm);
      set(bounds);
      cover(webview, scale);
      UIView::setAnimationsEnabled(enabled, mtm);
      cover(webview, 1.0);
    }
  }

  fn cover(webview: &WryWebView, scale: f64) {
    unsafe {
      webview.setTransform(CGAffineTransform {
        a: scale,
        b: 0.0,
        c: 0.0,
        d: scale,
        tx: 0.0,
        ty: 0.0,
      });
    }
  }

  fn showing(webview: &WryWebView) -> bool {
    unsafe {
      webview.window().is_some()
        && !webview.isHidden()
        && webview.URL().is_some()
        && !webview.isLoading()
        && webview.respondsToSelector(sel!(_doAfterNextPresentationUpdate:))
    }
  }

  fn wait_for_next_frame(webview: &WryWebView, mtm: MainThreadMarker) {
    let drawn = Rc::new(Cell::new(false));
    let told = drawn.clone();
    let block = RcBlock::new(move || told.set(true));
    let mode = NSString::from_str(MODE);
    if webview.respondsToSelector(sel!(_updateVisibleContentRects)) {
      let _ = objc2::exception::catch(AssertUnwindSafe(|| unsafe {
        let _: () = msg_send![webview, _updateVisibleContentRects];
      }));
    }
    unsafe {
      let _: () = msg_send![webview, _doAfterNextPresentationUpdate: &*block];
      CFRunLoop::main()
        .expect("The main thread has a run loop.")
        .add_common_mode(Some(&CFString::from_str(MODE)));
      let animated = UIView::areAnimationsEnabled(mtm);
      UIView::setAnimationsEnabled(false, mtm);
      let _: () = msg_send![class!(CATransaction), begin];
      let deadline = NSDate::dateWithTimeIntervalSinceNow(FRAME_PATIENCE.as_secs_f64());
      let run_loop = NSRunLoop::mainRunLoop();
      while !drawn.get() && deadline.timeIntervalSinceNow() > 0.0 {
        run_loop.runMode_beforeDate(&mode, &deadline);
      }
      let _: () = msg_send![class!(CATransaction), commit];
      UIView::setAnimationsEnabled(animated, mtm);
    }
    #[cfg(feature = "tracing")]
    if !drawn.get() {
      tracing::warn!(
        "The web view's frame at its new size was not drawn within {FRAME_PATIENCE:?}, so the resize is shown before it."
      );
    }
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  fn size(width: f64, height: f64) -> CGSize {
    CGSize::new(width, height)
  }

  #[test]
  fn the_same_size_changes_nothing() {
    assert_eq!(
      SizeChange::between(size(420.0, 912.0), size(420.0, 912.0), true),
      SizeChange::Unchanged
    );
    assert_eq!(
      SizeChange::between(size(0.0, 0.0), size(0.0, 0.0), true),
      SizeChange::Unchanged
    );
  }

  #[test]
  fn a_first_size_has_nothing_drawn_to_follow() {
    for (drawn, now) in [
      (size(0.0, 0.0), size(420.0, 912.0)),
      (size(0.0, 912.0), size(420.0, 912.0)),
      (size(420.0, 0.0), size(420.0, 912.0)),
      (size(420.0, 912.0), size(0.0, 0.0)),
      (size(420.0, 912.0), size(420.0, 0.0)),
    ] {
      assert_eq!(
        SizeChange::between(drawn, now, true),
        SizeChange::FirstSize,
        "{drawn:?} to {now:?}"
      );
    }
  }

  #[test]
  fn a_page_not_on_screen_is_not_waited_for() {
    assert_eq!(
      SizeChange::between(size(912.0, 420.0), size(420.0, 912.0), false),
      SizeChange::Offscreen
    );
  }

  #[test]
  fn a_page_on_screen_is_waited_for_in_either_direction() {
    for (drawn, now) in [
      (size(912.0, 420.0), size(420.0, 912.0)),
      (size(420.0, 912.0), size(912.0, 420.0)),
      (size(420.0, 912.0), size(420.0, 600.0)),
    ] {
      assert_eq!(
        SizeChange::between(drawn, now, true),
        SizeChange::OnScreen,
        "{drawn:?} to {now:?}"
      );
    }
  }

  #[test]
  fn a_page_turned_on_its_side_covers_the_upright_box_it_starts_in() {
    let scale = cover_scale(size(420.0, 912.0), size(912.0, 420.0)).expect("It is scaled.");
    assert!((scale - 912.0 / 420.0).abs() < 1e-12, "{scale}");
    assert!(scale * 420.0 >= 912.0 - 1e-9 && scale * 912.0 >= 420.0);
  }

  #[test]
  fn a_page_turned_upright_covers_the_sideways_box_it_starts_in() {
    let scale = cover_scale(size(912.0, 420.0), size(420.0, 912.0)).expect("It is scaled.");
    assert!((scale - 912.0 / 420.0).abs() < 1e-12, "{scale}");
    assert!(scale * 420.0 >= 912.0 - 1e-9 && scale * 912.0 >= 420.0);
  }

  #[test]
  fn a_page_covers_the_box_at_every_step_of_the_turn() {
    let from = size(912.0, 420.0);
    let to = size(420.0, 912.0);
    let scale = cover_scale(from, to).expect("It is scaled.");
    for step in 0..=100 {
      let progress = f64::from(step) / 100.0;
      let width = from.width + (to.width - from.width) * progress;
      let height = from.height + (to.height - from.height) * progress;
      let shown = scale + (1.0 - scale) * progress;
      assert!(
        to.width * shown >= width - 1e-9 && to.height * shown >= height - 1e-9,
        "{progress}"
      );
    }
  }

  #[test]
  fn no_smaller_scale_covers_the_box_at_any_step_of_the_turn() {
    for (from, to) in [
      (size(912.0, 420.0), size(420.0, 912.0)),
      (size(420.0, 912.0), size(912.0, 420.0)),
    ] {
      let scale = cover_scale(from, to).expect("It is scaled.");
      for step in 0..=100 {
        let progress = f64::from(step) / 100.0;
        let width = from.width + (to.width - from.width) * progress;
        let height = from.height + (to.height - from.height) * progress;
        let needed = (width / to.width).max(height / to.height);
        let shown = scale + (1.0 - scale) * progress;
        assert!(
          (shown - needed).abs() < 1e-9,
          "{from:?} to {to:?} at {progress}"
        );
      }
    }
  }

  #[test]
  fn a_page_already_covering_its_box_is_not_scaled() {
    for (from, to) in [
      (size(420.0, 912.0), size(420.0, 912.0)),
      (size(400.0, 800.0), size(420.0, 912.0)),
      (size(420.0, 600.0), size(420.0, 912.0)),
    ] {
      assert_eq!(cover_scale(from, to), None, "{from:?} to {to:?}");
    }
  }

  #[test]
  fn a_box_with_no_area_is_not_scaled_for() {
    for (from, to) in [
      (size(0.0, 912.0), size(912.0, 420.0)),
      (size(420.0, 0.0), size(912.0, 420.0)),
      (size(420.0, 912.0), size(0.0, 420.0)),
      (size(420.0, 912.0), size(912.0, 0.0)),
    ] {
      assert_eq!(cover_scale(from, to), None, "{from:?} to {to:?}");
    }
  }

  #[test]
  fn a_page_is_scaled_to_cover_only_while_the_resize_is_animated() {
    let from = size(912.0, 420.0);
    let to = size(420.0, 912.0);
    assert_eq!(turning_scale(from, to, true), cover_scale(from, to));
    assert!(turning_scale(from, to, true).is_some());
    assert_eq!(turning_scale(from, to, false), None);
  }

  #[test]
  fn a_page_already_covering_its_box_is_not_scaled_while_animated() {
    assert_eq!(
      turning_scale(size(420.0, 600.0), size(420.0, 912.0), true),
      None
    );
    assert_eq!(
      turning_scale(size(0.0, 0.0), size(420.0, 912.0), true),
      None
    );
  }

  #[test]
  fn only_a_page_on_screen_waits_for_its_frame() {
    assert!(SizeChange::OnScreen.waits_for_its_frame());
    for change in [
      SizeChange::Unchanged,
      SizeChange::FirstSize,
      SizeChange::Offscreen,
    ] {
      assert!(!change.waits_for_its_frame(), "{change:?}");
    }
  }
}
