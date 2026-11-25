use super::*;
use std::{cell::Cell, rc::Rc};
struct Fake {
    expired: bool,
    calls: Rc<Cell<usize>>,
    forbidden: bool,
}
impl Backend for Fake {
    fn name(&self) -> &'static str {
        "fake"
    }
    fn get_index(&self) -> io::Result<Option<Vec<u8>>> {
        self.calls.set(self.calls.get() + 1);
        if self.expired {
            Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                TokenExpired,
            ))
        } else if self.forbidden {
            Err(io::Error::new(io::ErrorKind::PermissionDenied, "HTTP 403"))
        } else {
            Ok(Some(b"index".to_vec()))
        }
    }
    fn put_index(&self, _: &[u8]) -> io::Result<()> {
        Ok(())
    }
    fn put_snapshot(&self, _: &str, _: &str, _: &[u8]) -> io::Result<()> {
        Ok(())
    }
    fn get_snapshot(&self, _: &str, _: &str) -> io::Result<Vec<u8>> {
        Ok(vec![])
    }
    fn list_snapshots(&self, _: &str) -> io::Result<Vec<String>> {
        Ok(vec![])
    }
}
#[test]
fn refreshes_once_on_401_but_not_403() {
    for forbidden in [false, true] {
        let calls = Rc::new(Cell::new(0));
        let refreshes = Rc::new(Cell::new(0));
        let count = refreshes.clone();
        let factory_calls = calls.clone();
        let backend = CredentialBackend {
            current: RefCell::new(Box::new(Fake {
                expired: !forbidden,
                forbidden,
                calls: calls.clone(),
            })),
            factory: Box::new(move |_| {
                Box::new(Fake {
                    expired: false,
                    forbidden: false,
                    calls: factory_calls.clone(),
                })
            }),
            refresh: Box::new(move || {
                count.set(count.get() + 1);
                Ok("new".into())
            }),
            name: "test",
        };
        if forbidden {
            assert!(backend.get_index().is_err());
            assert_eq!(refreshes.get(), 0);
            assert_eq!(calls.get(), 1);
        } else {
            assert_eq!(backend.get_index().unwrap(), Some(b"index".to_vec()));
            assert_eq!(refreshes.get(), 1);
            assert_eq!(calls.get(), 2);
        }
    }
}
#[test]
fn repeated_401_does_not_loop() {
    let calls = Rc::new(Cell::new(0));
    let factory_calls = calls.clone();
    let backend = CredentialBackend {
        current: RefCell::new(Box::new(Fake {
            expired: true,
            forbidden: false,
            calls: calls.clone(),
        })),
        factory: Box::new(move |_| {
            Box::new(Fake {
                expired: true,
                forbidden: false,
                calls: factory_calls.clone(),
            })
        }),
        refresh: Box::new(|| Ok("new".into())),
        name: "test",
    };
    assert!(backend.get_index().is_err());
    assert_eq!(calls.get(), 2);
}
