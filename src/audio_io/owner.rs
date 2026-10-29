//! A stream is created and destroyed on its owning thread. Dropping a lease never
//! joins a driver thread on the render worker. A failed/stale reply drops its lease.
use super::*;
use std::sync::{atomic::AtomicBool, mpsc};
pub struct StreamLease {
    stop: Option<mpsc::SyncSender<()>>,
    pub stopping: Arc<AtomicBool>,
}
impl Drop for StreamLease {
    fn drop(&mut self) {
        self.stopping.store(true, Ordering::Release);
        if let Some(tx) = self.stop.take() {
            let _ = tx.try_send(());
        }
    }
}
pub fn start<T: Send + 'static>(
    build: impl FnOnce(StreamLease) -> Result<(cpal::Stream, T)> + Send + 'static,
) -> Result<T> {
    let (tx, rx) = mpsc::sync_channel(1);
    let (stop, stopped) = mpsc::sync_channel(1);
    let lease = StreamLease {
        stop: Some(stop),
        stopping: Arc::new(AtomicBool::new(false)),
    };
    std::thread::Builder::new()
        .name("piratewave-device-owner".into())
        .spawn(move || {
            match build(lease) {
                Ok((stream, value)) => {
                    let _ = tx.send(Ok(value));
                    let _ = stopped.recv();
                    // Give output callbacks a short fade; destruction remains off-render.
                    std::thread::sleep(std::time::Duration::from_millis(20));
                    drop(stream);
                }
                Err(error) => {
                    let _ = tx.send(Err(error));
                }
            }
        })?;
    rx.recv()
        .map_err(|_| anyhow!("Device preparation worker stopped"))?
}
