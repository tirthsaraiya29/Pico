use cpal::Host;
use pico_core::error::{PicoError, Result};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{mpsc, OnceLock};

struct HostState {
    host: Host,
    streams: HashMap<u64, cpal::Stream>,
}

type HostJob = Box<dyn FnOnce(&mut HostState) + Send>;

static HOST_TX: OnceLock<mpsc::Sender<HostJob>> = OnceLock::new();
static NEXT_STREAM_ID: AtomicU64 = AtomicU64::new(1);

fn sender() -> &'static mpsc::Sender<HostJob> {
    HOST_TX.get_or_init(|| {
        let (tx, rx) = mpsc::channel::<HostJob>();
        // Keep one persistent thread owning `cpal::default_host()` for the process lifetime so
        // COM STA initialization (`ENUMERATOR` in WASAPI) never leaks across exited thread apartments.
        std::thread::Builder::new()
            .name("pico-audio-cpal-host".into())
            .spawn(move || {
                let mut state = HostState {
                    host: cpal::default_host(),
                    streams: HashMap::new(),
                };
                while let Ok(job) = rx.recv() {
                    job(&mut state);
                }
            })
            .expect("Failed to spawn pico-audio cpal host worker");
        tx
    })
}

pub(crate) fn with_host<F, R>(f: F) -> Result<R>
where
    F: FnOnce(&Host) -> Result<R> + Send + 'static,
    R: Send + 'static,
{
    let (done_tx, done_rx) = mpsc::channel();
    sender()
        .send(Box::new(move |state: &mut HostState| {
            let _ = done_tx.send(f(&state.host));
        }))
        .map_err(|_| PicoError::Audio("Audio host worker terminated".into()))?;
    done_rx
        .recv()
        .map_err(|_| PicoError::Audio("Audio host worker dropped response".into()))?
}

pub(crate) fn spawn_stream<F, R>(f: F) -> Result<(u64, R)>
where
    F: FnOnce(&Host) -> Result<(cpal::Stream, R)> + Send + 'static,
    R: Send + 'static,
{
    let (done_tx, done_rx) = mpsc::channel();
    sender()
        .send(Box::new(move |state: &mut HostState| {
            let res = f(&state.host).map(|(stream, extra)| {
                let id = NEXT_STREAM_ID.fetch_add(1, Ordering::Relaxed);
                state.streams.insert(id, stream);
                (id, extra)
            });
            let _ = done_tx.send(res);
        }))
        .map_err(|_| PicoError::Audio("Audio host worker terminated".into()))?;
    done_rx
        .recv()
        .map_err(|_| PicoError::Audio("Audio host worker dropped response".into()))?
}

pub(crate) fn drop_stream(stream_id: u64) {
    if stream_id == 0 {
        return;
    }
    if let Some(tx) = HOST_TX.get() {
        let _ = tx.send(Box::new(move |state: &mut HostState| {
            state.streams.remove(&stream_id);
        }));
    }
}
