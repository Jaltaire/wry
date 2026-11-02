// Copyright 2020-2023 Tauri Programme within The Commons Conservancy
// SPDX-License-Identifier: Apache-2.0
// SPDX-License-Identifier: MIT

use std::borrow::Cow;

use http::{Response, StatusCode};

pub(crate) fn answer_unless_panicked(
  handle: impl FnOnce() -> Option<Response<Cow<'static, [u8]>>>,
) -> Option<Response<Cow<'static, [u8]>>> {
  match std::panic::catch_unwind(std::panic::AssertUnwindSafe(handle)) {
    Ok(response) => response,
    Err(_) => {
      #[cfg(feature = "tracing")]
      tracing::error!(
        "A custom protocol handler panicked, so its request is answered with an error."
      );
      let mut failed = Response::new(Cow::Borrowed(&[][..]));
      *failed.status_mut() = StatusCode::INTERNAL_SERVER_ERROR;
      Some(failed)
    }
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn a_handler_that_answers_is_answered_as_it_answered() {
    let answered = answer_unless_panicked(|| Some(Response::new(Cow::Borrowed(&b"page"[..]))))
      .expect("The handler answered.");
    assert_eq!(answered.status(), StatusCode::OK);
    assert_eq!(answered.body().as_ref(), b"page");
  }

  #[test]
  fn a_handler_that_declines_is_declined() {
    assert!(answer_unless_panicked(|| None).is_none());
  }

  #[test]
  fn a_handler_that_panics_fails_its_request_rather_than_unwinding_further() {
    let failed = answer_unless_panicked(|| panic!("The handler found something already borrowed."))
      .expect("A panicked handler's request is still answered.");
    assert_eq!(failed.status(), StatusCode::INTERNAL_SERVER_ERROR);
    assert!(failed.body().is_empty());
  }
}
