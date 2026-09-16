//! Composer attachments: file paths the next message carries. Images are
//! detected by extension — they render as thumbnail chips and go to the
//! backend as image inputs (codex `localImage`, ACP `resource_link`) instead
//! of only a path mention. Attach via the picker, drag-drop, or paste —
//! clipboard images are written under the project's store dir so the backend
//! gets a real path. Dropped/pasted files route by kind: images attach as
//! chips, anything else lands in the draft as an `@path` mention — the same
//! reference the explorer click and @-picker produce.

use gpui_kit::*;

use crate::workspace::Workspace;

/// Extensions treated as image attachments — the set codex's `localImage`
/// input and the thumbnail chip both accept.
const IMAGE_EXTENSIONS: [&str; 5] = ["png", "jpg", "jpeg", "gif", "webp"];

/// True when `path` names an image file by extension (case-insensitive).
pub(crate) fn is_image_path(path: &str) -> bool {
    std::path::Path::new(path)
        .extension()
        .is_some_and(|e| IMAGE_EXTENSIONS.contains(&e.to_string_lossy().to_lowercase().as_str()))
}

/// The image attachments in `attachments` as paths — what `turn/start`'s
/// `localImage` inputs and ACP `resource_link` blocks carry.
pub(crate) fn image_paths(attachments: &[SharedString]) -> Vec<std::path::PathBuf> {
    attachments
        .iter()
        .filter(|a| is_image_path(a.as_str()))
        .map(|a| std::path::PathBuf::from(a.as_str()))
        .collect()
}

/// Where an incoming (dropped or pasted) path lands in the composer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum AttachRoute {
    /// Image files attach as chips — they ride the backend's image inputs.
    Chip(std::path::PathBuf),
    /// Everything else becomes an `@path` mention in the draft text. Paths
    /// under `root` relativize so the mention matches the @-picker's format.
    Mention(String),
}

/// Classify incoming paths: images → chips, other files → mentions.
pub(crate) fn route_attach_paths(root: &std::path::Path, paths: &[std::path::PathBuf]) -> Vec<AttachRoute> {
    paths
        .iter()
        .map(|path| {
            if is_image_path(&path.to_string_lossy()) {
                return AttachRoute::Chip(path.clone());
            }
            let mention = match path.strip_prefix(root) {
                Ok(rel) if !rel.as_os_str().is_empty() => rel.to_string_lossy().into_owned(),
                _ => path.to_string_lossy().into_owned(),
            };
            AttachRoute::Mention(mention)
        })
        .collect()
}

/// Write a pasted clipboard image under the project's store dir and return
/// its path — backends address attachments by path, so the bytes need a
/// file. `image.id` (a content hash) keeps rapid pastes distinct.
fn save_clipboard_image(dir: &std::path::Path, image: &Image) -> Option<std::path::PathBuf> {
    if std::fs::create_dir_all(dir).is_err() {
        return None;
    }
    let path = dir.join(format!("paste-{}-{}.{}", std::process::id(), image.id, image.format().extension()));
    std::fs::write(&path, image.bytes()).ok().map(|_| path)
}

impl Workspace {
    /// Open the native file picker and attach the chosen files.
    pub fn attach_file(&mut self, cx: &mut Context<Self>) {
        let rx = cx.prompt_for_paths(gpui_kit::PathPromptOptions {
            files: true,
            directories: false,
            multiple: true,
            prompt: Some("Attach files".into()),
        });
        cx.spawn(async move |this, cx| {
            let Ok(Ok(Some(paths))) = rx.await else { return };
            let _ = this.update(cx, |this, cx| this.add_attachments(paths, cx));
        })
        .detach();
    }

    /// Route dropped/pasted paths: images attach as chips, other files
    /// append `@path` mentions to the draft (project-relative when the file
    /// lives under the root). Mentions focus the composer so the user sees
    /// where the reference landed.
    pub(crate) fn attach_incoming(&mut self, paths: Vec<std::path::PathBuf>, window: &mut Window, cx: &mut Context<Self>) {
        let mut chips = Vec::new();
        let mut mentions = Vec::new();
        for route in route_attach_paths(self.project.root(), &paths) {
            match route {
                AttachRoute::Chip(path) => chips.push(path),
                AttachRoute::Mention(path) => mentions.push(path),
            }
        }
        if !mentions.is_empty() {
            let mut text = self.composer.read(cx).value().to_string();
            for path in &mentions {
                text = crate::views::explorer::mention_text(&text, path);
            }
            self.composer.update(cx, |s, cx| {
                s.set_value(text, window, cx);
                s.focus(window, cx);
            });
            // `set_value` suppresses Change — nudge so the mention menu closes.
            cx.notify();
        }
        // Chips attach after the composer update: focusing the input stashes
        // the outgoing chat's draft state, which would drop fresh chips.
        if !chips.is_empty() {
            self.add_attachments(chips, cx);
        }
    }

    /// Append unique file paths to the active chat's attachments.
    pub(crate) fn add_attachments(&mut self, paths: Vec<std::path::PathBuf>, cx: &mut Context<Self>) {
        let chat = &mut self.chats[self.active];
        for name in paths.iter().map(|p| p.to_string_lossy().into_owned()) {
            if !chat.attachments.iter().any(|a| a.as_str() == name) {
                chat.attachments.push(name.into());
            }
        }
        cx.notify();
    }

    pub fn remove_attachment(&mut self, ix: usize, cx: &mut Context<Self>) {
        let attachments = &mut self.chats[self.active].attachments;
        if ix < attachments.len() {
            attachments.remove(ix);
        }
        cx.notify();
    }

    /// Capture-phase `Paste` on the composer: clipboard images are saved to
    /// disk and attached, copied file paths route like dropped ones (images
    /// as chips, the rest as mentions) — both stop the paste so no raw path
    /// text lands in the input. A plain-text clipboard propagates to the
    /// input's own paste handler untouched.
    pub(crate) fn paste_attachments(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(item) = cx.read_from_clipboard() else { return };
        let dir = self.project.dir().join("attachments");
        let mut paths: Vec<std::path::PathBuf> = Vec::new();
        for entry in item.entries() {
            match entry {
                ClipboardEntry::Image(image) => {
                    paths.extend(save_clipboard_image(&dir, image));
                },
                ClipboardEntry::ExternalPaths(external) => paths.extend(external.0.iter().cloned()),
                ClipboardEntry::String(_) => {},
            }
        }
        if paths.is_empty() {
            return;
        }
        cx.stop_propagation();
        self.attach_incoming(paths, window, cx);
    }
}

#[cfg(test)]
mod tests {
    use super::{AttachRoute, image_paths, is_image_path, route_attach_paths};
    use std::path::{Path, PathBuf};

    #[test]
    fn image_extensions_detect_images() {
        for path in ["a.png", "b.JPG", "c.jpeg", "d.gif", "e.webp", "/tmp/shot.PNG"] {
            assert!(is_image_path(path), "{path}");
        }
        for path in ["a.rs", "b.txt", "c.png.bak", "no-ext", ".png", "d.svg"] {
            assert!(!is_image_path(path), "{path}");
        }
    }

    #[test]
    fn image_paths_filters_to_images() {
        let all = vec!["/tmp/a.png".into(), "/tmp/b.rs".into(), "/tmp/c.webp".into()];
        let images = image_paths(&all);
        assert_eq!(images, vec![PathBuf::from("/tmp/a.png"), PathBuf::from("/tmp/c.webp")]);
        assert!(image_paths(&[]).is_empty());
    }

    #[test]
    fn route_attach_paths_splits_images_from_mentions() {
        let root = Path::new("/repo");
        let routes = route_attach_paths(
            root,
            &[
                PathBuf::from("/repo/src/main.rs"),
                PathBuf::from("/repo/shot.png"),
                PathBuf::from("/elsewhere/notes.txt"),
                PathBuf::from("/elsewhere/pic.JPEG"),
            ],
        );
        assert_eq!(
            routes,
            vec![
                AttachRoute::Mention("src/main.rs".into()),
                AttachRoute::Chip(PathBuf::from("/repo/shot.png")),
                AttachRoute::Mention("/elsewhere/notes.txt".into()),
                AttachRoute::Chip(PathBuf::from("/elsewhere/pic.JPEG")),
            ]
        );
    }

    #[test]
    fn route_attach_paths_edge_cases() {
        let root = Path::new("/repo");
        // A path that IS the root can't relativize to a usable mention —
        // keep the absolute form rather than a bare "@".
        assert_eq!(route_attach_paths(root, &[PathBuf::from("/repo")]), vec![AttachRoute::Mention("/repo".into())]);
        // Extension-less and non-image files mention; empty input routes nothing.
        assert_eq!(route_attach_paths(root, &[PathBuf::from("/repo/Makefile")]), vec![AttachRoute::Mention("Makefile".into())]);
        assert!(route_attach_paths(root, &[]).is_empty());
    }
}
