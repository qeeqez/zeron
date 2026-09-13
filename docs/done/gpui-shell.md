# GPUI app shell — DONE

- `gpui-kit = "0.6"` single dep (gpui-pre 0.3.4 + gpui-base + gpui-component + gpui-kit-assets)
- `gpui_kit::application()` + `gpui_kit::init(cx)` + `Root`-wrapped window
- `Workspace` view: sidebar + chat + composer + agents panel, `cx.listener` handlers
- Note: earlier `gpui-unofficial` 1.20.0-pre attempt hit the `gpui-apple` sibling-dir build bug; gpui-pre inlines the backend, no workaround needed.
