//! Composer attachments: file paths the next message carries. Images are
//! detected by extension — they render as thumbnail chips and go to the
//! backend as image inputs (codex `localImage`, ACP `resource_link`) instead
//! of only a path mention. Attach via the picker, drag-drop, or paste —
//! clipboard images are written under the project's store dir so the backend
//! gets a real path.

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
    /// disk and attached, copied file paths attach as chips — both stop the
    /// paste so no raw path text lands in the input. A plain-text clipboard
    /// propagates to the input's own paste handler untouched.
    pub(crate) fn paste_attachments(&mut self, cx: &mut Context<Self>) {
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
        self.add_attachments(paths, cx);
    }
}

#[cfg(test)]
mod tests {
    use super::{image_paths, is_image_path};

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
        assert_eq!(images, vec![std::path::PathBuf::from("/tmp/a.png"), std::path::PathBuf::from("/tmp/c.webp")]);
        assert!(image_paths(&[]).is_empty());
    }
}
