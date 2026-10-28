---
"wry": minor
---

On Android, a web view whose page is lost is replaced by one made with the same attributes and id, in the activity it was in, and the new `WebViewBuilderExtAndroid::with_on_page_lost_handler` is told how the page went, so the application can load its page again. A page is lost when the web view's renderer process crashes or is ended by the system, which no longer takes the application with it, and when the activity the web view was in is destroyed and created again in the same process, as Android does to show a configuration change the activity does not handle itself. With the handler set, nothing is loaded into the replacement until the application asks for its page; without it, the replacement loads what the lost web view was made to load.
