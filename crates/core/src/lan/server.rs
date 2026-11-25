//! Owned LAN listener: synchronous bind, bounded clients and explicit shutdown.
use super::*;
use std::sync::{
    atomic::{AtomicBool, AtomicUsize, Ordering},
    Arc, RwLock,
};

pub struct LanServer {
    pub port: u16,
    pin: Arc<RwLock<String>>,
    stop: Arc<AtomicBool>,
    task: Option<std::thread::JoinHandle<()>>,
}

impl LanServer {
    pub fn start(store: PathBuf, port: u16, pin: &str) -> io::Result<Self> {
        let listener = TcpListener::bind(("0.0.0.0", port))?;
        let port = listener.local_addr()?.port();
        listener.set_nonblocking(true)?;
        let stop = Arc::new(AtomicBool::new(false));
        let expected_pin = Arc::new(RwLock::new(pin_hash(pin)));
        let active = Arc::new(AtomicUsize::new(0));
        let task = {
            let stop = stop.clone();
            let pin = expected_pin.clone();
            std::thread::spawn(move || {
                while !stop.load(Ordering::Relaxed) {
                    match listener.accept() {
                        Ok((stream, _)) => {
                            if active.load(Ordering::Relaxed) >= 8 {
                                continue;
                            }
                            active.fetch_add(1, Ordering::Relaxed);
                            let active = active.clone();
                            let store = store.clone();
                            let hash = pin.read().unwrap().clone();
                            std::thread::spawn(move || {
                                let _ = handle_conn(stream, &store, &hash);
                                active.fetch_sub(1, Ordering::Relaxed);
                            });
                        }
                        Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                            std::thread::sleep(Duration::from_millis(20))
                        }
                        Err(_) => break,
                    }
                }
            })
        };
        Ok(Self {
            port,
            pin: expected_pin,
            stop,
            task: Some(task),
        })
    }

    pub fn set_pin(&self, pin: &str) {
        *self.pin.write().unwrap() = pin_hash(pin);
    }
}

impl Drop for LanServer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(task) = self.task.take() {
            let _ = task.join();
        }
    }
}
