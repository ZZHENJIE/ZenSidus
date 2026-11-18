//! Sound effects on a dedicated thread: `AppState::play_audio` sends an
//! [`AppAudio`] down a channel and the thread decodes the embedded clip into
//! the default output device.

use rodio::{Decoder, DeviceSinkBuilder};
use std::io::Cursor;
use std::sync::mpsc::{Sender, channel};
use std::thread;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AppAudio {
    Hint,
}

impl AppAudio {
    /// Embedded asset bytes (see `utils::Assets`); no filesystem access, so
    /// playback works no matter the working directory.
    fn data(self) -> std::borrow::Cow<'static, [u8]> {
        match self {
            Self::Hint => {
                crate::utils::Assets::get("audios/hint.wav")
                    .expect("embedded audio missing: audios/hint.wav")
                    .data
            }
        }
    }
}

/// Start the playback thread and hand back its send end. The thread exits
/// when every sender is dropped.
pub fn audio_thread() -> Sender<AppAudio> {
    let (tx, rx): (Sender<AppAudio>, std::sync::mpsc::Receiver<AppAudio>) = channel();
    thread::spawn(move || {
        let mut handle = DeviceSinkBuilder::open_default_sink().expect("open default audio stream");
        handle.log_on_drop(false);

        while let Ok(audio) = rx.recv() {
            let cursor = Cursor::new(audio.data());
            let source = match Decoder::<Cursor<std::borrow::Cow<'static, [u8]>>>::try_from(cursor)
            {
                Ok(source) => source,
                Err(err) => {
                    tracing::warn!(
                        "{}",
                        crate::tr!(
                            "app.log.audio_decode_failed",
                            audio = format!("{audio:?}"),
                            err = err.to_string()
                        )
                    );
                    continue;
                }
            };
            handle.mixer().add(source);
        }
    });
    tx
}
