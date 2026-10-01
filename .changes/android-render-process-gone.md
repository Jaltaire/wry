---
"wry": minor
---

On Android, a web view whose renderer process crashes or is ended by the system no longer takes the application with it. The web view is destroyed and replaced by a blank one with the same attributes and id, and the new `WebViewBuilderExtAndroid::with_on_render_process_gone_handler` is told whether the process crashed or was killed, so the application can load its page again.
