//! Inline playback loaded on demand, with a bounded number of active decoders.
use adw::prelude::*;
use gtk::{gio, glib};
use handover_core::{Attachment, AttachmentKind};
use std::{
    cell::RefCell,
    fs,
    os::fd::AsRawFd,
    path::PathBuf,
    rc::{Rc, Weak},
};

const MAX_PLAYBACK_BYTES: u64 = 50 * 1024 * 1024;
thread_local! { static PLAYERS:RefCell<Vec<Weak<Loaded>>> = const { RefCell::new(Vec::new()) }; }
type Slot = RefCell<Option<Rc<Loaded>>>;
struct Loaded {
    owner: Weak<Slot>,
    stream: gtk::MediaFile,
    controls: Option<gtk::MediaControls>,
    video: Option<gtk::Picture>,
    source: RefCell<Option<fs::File>>,
    content: glib::WeakRef<gtk::Box>,
    start: glib::WeakRef<gtk::Button>,
    status: glib::WeakRef<gtk::Label>,
    spinner: glib::WeakRef<gtk::Spinner>,
}
impl Loaded {
    fn stop(&self) {
        if self.source.borrow().is_none() {
            return;
        }
        if let Some(controls) = &self.controls {
            controls.set_media_stream(None::<&gtk::MediaStream>);
        }
        if let Some(video) = &self.video {
            video.set_paintable(None::<&gtk::gdk::Paintable>);
        }
        // Clearing closes the backend and stops playback. Do not enqueue a
        // separate pause immediately before destroying its player.
        self.stream.clear();
        self.source.borrow_mut().take();
        if let Some(status) = self.status.upgrade() {
            status.set_label("Stopped");
        }
        if let Some(spinner) = self.spinner.upgrade() {
            spinner.stop();
            spinner.set_visible(false);
        }

        if let Some(content) = self.content.upgrade() {
            while let Some(child) = content.first_child() {
                content.remove(&child);
            }
        }
        if let Some(start) = self.start.upgrade() {
            start.set_visible(true);
            start.set_sensitive(true);
            start.set_label("Play media");
        }
        if let Some(owner) = self.owner.upgrade() {
            let same = owner
                .borrow()
                .as_ref()
                .is_some_and(|player| std::ptr::eq(player.as_ref(), self));
            if same {
                let previous = owner.borrow_mut().take();
                drop(previous);
            }
        }
    }
}
impl Drop for Loaded {
    fn drop(&mut self) {
        self.stop();
    }
}
fn make_room() {
    let previous = PLAYERS.with(|players| {
        let mut players = players.borrow_mut();
        players.retain(|player| {
            player
                .upgrade()
                .is_some_and(|player| player.stream.file().is_some())
        });
        if players.len() >= 4 {
            players.remove(0).upgrade()
        } else {
            None
        }
    });
    if let Some(previous) = previous {
        previous.stop();
    }
}

pub fn widget(attachment: &Attachment, _parent: &adw::ApplicationWindow) -> gtk::Widget {
    let root = gtk::Box::new(gtk::Orientation::Vertical, 6);
    root.add_css_class("inline-media");
    root.set_width_request(240);
    let video_kind = AttachmentKind::of_mime(attachment.mime.as_deref()) == AttachmentKind::Video;
    root.add_css_class(if video_kind {
        "video-attachment"
    } else {
        "audio-attachment"
    });
    let title = gtk::Label::new(Some(attachment.name.as_deref().unwrap_or(if video_kind {
        "Video attachment"
    } else {
        "Audio attachment"
    })));
    title.set_xalign(0.0);
    title.set_max_width_chars(32);
    title.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
    root.append(&title);
    let Some(path) = playback_path(attachment) else {
        let unavailable = gtk::Box::new(gtk::Orientation::Vertical, 4);
        show_fallback(
            &unavailable,
            attachment,
            None,
            "This media file is not available locally.",
        );
        root.append(&unavailable);
        return root.upcast();
    };
    let start = gtk::Button::with_label("Play media");
    start.set_tooltip_text(Some("Load and play this attachment"));
    root.append(&start);
    let activity = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    let spinner = gtk::Spinner::new();
    spinner.set_visible(false);
    let status = gtk::Label::new(None);
    status.set_xalign(0.0);
    status.add_css_class("playback-state");
    status.add_css_class("caption");
    activity.append(&spinner);
    activity.append(&status);
    root.append(&activity);
    let content = gtk::Box::new(gtk::Orientation::Vertical, 4);
    root.append(&content);
    let failure = gtk::Box::new(gtk::Orientation::Vertical, 4);
    failure.set_visible(false);
    root.append(&failure);
    let slot: Rc<Slot> = Rc::new(RefCell::new(None));
    let current = slot.clone();
    let content_weak = content.downgrade();
    let failure_weak = failure.downgrade();
    let part = attachment.clone();
    let spinner_weak = spinner.downgrade();
    let status_weak = status.downgrade();
    start.connect_clicked(move |start| {
        let previous = current.borrow_mut().take();
        drop(previous);
        let Some(content) = content_weak.upgrade() else {
            return;
        };
        let Some(failure) = failure_weak.upgrade() else {
            return;
        };
        failure.set_visible(false);
        let Some(spinner) = spinner_weak.upgrade() else {
            return;
        };
        let Some(status) = status_weak.upgrade() else {
            return;
        };
        status.set_visible(true);
        let Some(source) = open_media(&path) else {
            show_fallback(
                &failure,
                &part,
                None,
                "This media file is not available locally.",
            );
            return;
        };
        make_room();
        let file = gio::File::for_path(format!("/proc/self/fd/{}", source.as_raw_fd()));
        let stream = gtk::MediaFile::for_file(&file);
        if stream.error().is_some() {
            show_fallback(
                &failure,
                &part,
                Some(&gio::File::for_path(&path)),
                "This media format could not be played here.",
            );
            return;
        }
        #[cfg(test)]
        stream.set_muted(true);
        let video = if video_kind {
            let picture = gtk::Picture::for_paintable(&stream);
            picture.set_content_fit(gtk::ContentFit::Contain);
            picture.set_size_request(240, 160);
            picture.add_css_class("video-surface");
            content.append(&picture);
            Some(picture)
        } else {
            None
        };
        let controls = gtk::MediaControls::new(Some(&stream));
        controls.add_css_class("playback-controls");
        content.append(&controls);
        let transport = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        let back = gtk::Button::from_icon_name("media-seek-backward-symbolic");
        back.set_tooltip_text(Some("Back 10 seconds"));
        back.add_css_class("seek-back");
        let forward = gtk::Button::from_icon_name("media-seek-forward-symbolic");
        forward.set_tooltip_text(Some("Forward 10 seconds"));
        forward.add_css_class("seek-forward");
        let stop = gtk::Button::from_icon_name("media-playback-stop-symbolic");
        stop.set_tooltip_text(Some("Stop and unload media"));
        stop.add_css_class("stop-playback");
        for button in [&back, &forward, &stop] {
            button.add_css_class("flat");
            transport.append(button);
        }
        content.append(&transport);
        for (button, delta) in [(&back, -10_000_000i64), (&forward, 10_000_000i64)] {
            let weak = stream.downgrade();
            button.connect_clicked(move |_| {
                if let Some(stream) = weak.upgrade() {
                    seek_by(&stream, delta);
                }
            });
        }
        let update: Rc<dyn Fn(&gtk::MediaFile)> = {
            let status = status.downgrade();
            let spinner = spinner.downgrade();
            let back = back.downgrade();
            let forward = forward.downgrade();
            Rc::new(move |stream| {
                if stream.file().is_none() {
                    return;
                }
                let busy = !stream.is_prepared() || stream.is_seeking();
                if let Some(spinner) = spinner.upgrade() {
                    spinner.set_spinning(busy && stream.error().is_none());
                    spinner.set_visible(busy && stream.error().is_none());
                }
                let seekable = stream.is_prepared()
                    && stream.is_seekable()
                    && stream.duration() > 0
                    && !stream.is_seeking();
                for button in [&back, &forward] {
                    if let Some(button) = button.upgrade() {
                        button.set_sensitive(seekable);
                    }
                }
                if let Some(status) = status.upgrade() {
                    let text = if stream.error().is_some() {
                        "Unable to play this media".into()
                    } else if !stream.is_prepared() {
                        "Loading media…".into()
                    } else if stream.is_seeking() {
                        "Seeking…".into()
                    } else {
                        let state = if stream.is_ended() {
                            "Finished"
                        } else if stream.is_playing() {
                            "Playing"
                        } else {
                            "Paused"
                        };
                        let position = time_label(stream.timestamp());
                        if stream.duration() > 0 {
                            format!("{state} · {position} / {}", time_label(stream.duration()))
                        } else {
                            format!("{state} · {position}")
                        }
                    };
                    status.set_label(&text);
                }
            })
        };
        update(&stream);
        stream.connect_notify_local(None, move |stream, _| update(stream));
        let loaded = Rc::new(Loaded {
            owner: Rc::downgrade(&current),
            stream: stream.clone(),
            controls: Some(controls.clone()),
            video,
            source: RefCell::new(Some(source)),
            content: content.downgrade(),
            start: start.downgrade(),
            status: status.downgrade(),
            spinner: spinner.downgrade(),
        });
        PLAYERS.with(|players| players.borrow_mut().push(Rc::downgrade(&loaded)));
        let weak = Rc::downgrade(&loaded);
        stop.connect_clicked(move |_| {
            if let Some(player) = weak.upgrade() {
                player.stop();
            }
        });
        let keys = gtk::EventControllerKey::new();
        let weak = Rc::downgrade(&loaded);
        keys.connect_key_pressed(move |_, key, _, _| {
            let Some(player) = weak.upgrade() else {
                return glib::Propagation::Proceed;
            };
            let stream = &player.stream;
            match key {
                gtk::gdk::Key::space if stream.is_prepared() => {
                    if stream.is_playing() {
                        stream.pause();
                    } else {
                        stream.play();
                    }
                }
                gtk::gdk::Key::Left => seek_by(stream, -5_000_000),
                gtk::gdk::Key::Right => seek_by(stream, 5_000_000),
                _ => return glib::Propagation::Proceed,
            }
            glib::Propagation::Stop
        });
        controls.add_controller(keys);
        *current.borrow_mut() = Some(loaded);
        let button = start.downgrade();
        stream.connect_prepared_notify(move |stream| {
            if stream.file().is_none() {
                return;
            }
            if let Some(button) = button.upgrade() {
                button.set_visible(!stream.is_prepared());
            }
        });
        let failure = failure.downgrade();
        let button = start.downgrade();
        let part = part.clone();
        let file = gio::File::for_path(&path);
        let current_player = Rc::downgrade(&current);
        let error_status = status.downgrade();
        stream.connect_error_notify(move |stream| {
            if stream.error().is_some() && stream.file().is_some() {
                if let Some(slot) = current_player.upgrade() {
                    let player = slot.borrow_mut().take();
                    if let Some(player) = player {
                        player.stop();
                    }
                }
                if let Some(status) = error_status.upgrade() {
                    status.set_visible(false);
                }
                if let Some(failure) = failure.upgrade() {
                    show_fallback(
                        &failure,
                        &part,
                        Some(&file),
                        "This media format could not be played here.",
                    );
                }
                if let Some(button) = button.upgrade() {
                    button.set_visible(true);
                    button.set_sensitive(true);
                    button.set_label("Play media");
                }
            }
        });
        start.set_label("Loading media…");
        start.set_sensitive(false);
        if stream.is_prepared() {
            start.set_visible(false);
        }
        stream.play();
    });
    let current = Rc::downgrade(&slot);
    root.connect_unmap(move |_| {
        if let Some(slot) = current.upgrade() {
            let player = slot.borrow_mut().take();
            if let Some(player) = player {
                player.stop();
            }
        }
    });
    let current = Rc::downgrade(&slot);
    root.connect_parent_notify(move |root| {
        if root.parent().is_none()
            && let Some(slot) = current.upgrade()
        {
            let player = slot.borrow_mut().take();
            if let Some(player) = player {
                player.stop();
            }
        }
    });
    root.upcast()
}

fn seek_by(stream: &impl IsA<gtk::MediaStream>, delta: i64) {
    let stream = stream.as_ref();
    if stream.is_prepared() && stream.is_seekable() && stream.duration() > 0 && !stream.is_seeking()
    {
        stream.seek(
            stream
                .timestamp()
                .saturating_add(delta)
                .clamp(0, stream.duration()),
        );
    }
}

fn time_label(microseconds: i64) -> String {
    let seconds = microseconds.max(0) / 1_000_000;
    if seconds >= 3600 {
        format!(
            "{}:{:02}:{:02}",
            seconds / 3600,
            seconds / 60 % 60,
            seconds % 60
        )
    } else {
        format!("{}:{:02}", seconds / 60, seconds % 60)
    }
}

fn open_media(path: &std::path::Path) -> Option<std::fs::File> {
    use std::os::unix::fs::OpenOptionsExt;
    let source = fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)
        .ok()?;
    let metadata = source.metadata().ok()?;
    if !metadata.is_file() || metadata.len() > MAX_PLAYBACK_BYTES {
        return None;
    }
    Some(source)
}

fn playback_path(attachment: &Attachment) -> Option<PathBuf> {
    if !matches!(
        AttachmentKind::of_mime(attachment.mime.as_deref()),
        AttachmentKind::Audio | AttachmentKind::Video
    ) {
        return None;
    }
    let path = PathBuf::from(attachment.staged_path.as_deref()?);
    let metadata = fs::symlink_metadata(&path).ok()?;
    if !metadata.file_type().is_file() || metadata.len() > MAX_PLAYBACK_BYTES {
        return None;
    }
    Some(path)
}

fn show_fallback(
    container: &gtk::Box,
    attachment: &Attachment,
    file: Option<&gio::File>,
    message: &str,
) {
    // The function can be called by a decoder error after the initial state;
    // remove prior fallback contents before rebuilding the actions.
    while let Some(child) = container.first_child() {
        container.remove(&child);
    }
    let label = gtk::Label::new(Some(message));
    label.set_xalign(0.0);
    label.set_wrap(true);
    container.append(&label);

    if let Some(file) = file {
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        let button = gtk::Button::with_label("Open externally");
        let uri = file.uri();
        button.connect_clicked(move |_| {
            let _ = gio::AppInfo::launch_default_for_uri(&uri, None::<&gio::AppLaunchContext>);
        });
        row.append(&button);
        let name = attachment
            .name
            .as_deref()
            .filter(|name| !name.trim().is_empty());
        if let Some(name) = name {
            let title = gtk::Label::new(Some(name));
            title.set_xalign(0.0);
            title.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
            title.set_hexpand(true);
            row.append(&title);
        }
        container.append(&row);
    }
    container.set_visible(true);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::symlink;

    fn attachment(path: &std::path::Path, mime: &str) -> Attachment {
        Attachment {
            local_id: "test".into(),
            mime: Some(mime.into()),
            name: None,
            size_bytes: None,
            staged_path: Some(path.to_string_lossy().into_owned()),
        }
    }

    #[test]
    fn allows_small_regular_audio_and_video_files() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sample.media");
        fs::write(&path, b"media").unwrap();
        assert!(
            open_media(&path).is_some(),
            "verified regular files must open as pinned file descriptors"
        );

        assert_eq!(
            playback_path(&attachment(&path, "audio/ogg")),
            Some(path.clone())
        );
        assert_eq!(playback_path(&attachment(&path, "video/mp4")), Some(path));
    }

    #[test]
    fn rejects_remote_missing_symlink_directory_and_large_files() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("target.media");
        fs::write(&target, b"media").unwrap();
        let link = dir.path().join("link.media");
        symlink(&target, &link).unwrap();
        assert!(playback_path(&attachment(&link, "audio/ogg")).is_none());
        assert!(playback_path(&attachment(dir.path(), "video/mp4")).is_none());
        assert!(playback_path(&attachment(&dir.path().join("missing"), "audio/ogg")).is_none());
        assert!(playback_path(&attachment(&target, "image/png")).is_none());

        let large = dir.path().join("large.media");
        fs::File::create(&large)
            .unwrap()
            .set_len(MAX_PLAYBACK_BYTES + 1)
            .unwrap();
        assert!(playback_path(&attachment(&large, "video/mp4")).is_none());
    }
}
