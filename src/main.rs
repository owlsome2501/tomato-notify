use std::cell::RefCell;
use std::io;
use std::io::prelude::*;
use std::thread;
use std::os::unix::net::{UnixListener, UnixStream};
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

static UNIX_ADDRESS: &'static str = "/tmp/tomato-notity-socket";

struct Reporter {
    listener: UnixListener,
    control: Arc<AtomicBool>,
}

impl Reporter {
    fn new(control: Arc<AtomicBool>) -> io::Result<Reporter> {
        let reporter = Reporter {
            listener: UnixListener::bind(UNIX_ADDRESS)?,
            control,
        };
        Ok(reporter)
    }

    fn report(mut stream: UnixStream) {
        stream.write_all(b"hello").unwrap();
    }

    fn run(self) -> thread::JoinHandle<()> {
        thread::spawn( move || {
            for stream in self.listener.incoming() {
                match stream {
                    Ok(stream) => {
                        Reporter::report(stream);
                    }
                    Err(e) => eprintln!("accept stream failed: {}", e),
                }
                if ! self.control.load(Ordering::SeqCst) {
                    break;
                }
            }
        })
    }
}

#[allow(unused)]
struct TomatoClock {
    status: u32,
}

#[allow(unused)]
struct Notifier {
    backend: RefCell<Command>,
}

impl Notifier {
    fn new() -> Notifier {
        let backend = "dunstify";
        Notifier {
            backend: RefCell::new(Command::new(backend)),
        }
    }

    fn notify(&self, msg: &str, actions: &[(&str, &str)]) -> Result<String, ()> {
        let actions: Vec<String> = actions
            .iter()
            .map(|(action, label)| format!("--action={},{}", action, label))
            .collect();
        let mut args: Vec<String> = Vec::new();
        args.extend(actions);
        args.push(msg.to_string());
        let output = self.backend.borrow_mut().args(&args).output().unwrap();
        if output.status.success() {
            let output = String::from_utf8(output.stdout).unwrap().trim().to_string();
            if output != "2" {
                Ok(output)
            } else {
                Err(())
            }
        } else {
            Err(())
        }
    }
}

fn main() {
    // let notifier = Notifier::new();
    // if let Ok(selection) = notifier.notify("help me", &[("ok","Ok"),("like","Like")]) {
    //     println!("{}", selection);
    // };
    // if let Ok(selection) = notifier.notify("good me", &[("ok","Ok"),("like","Like")]) {
    //     println!("{}", selection);
    // };

    let running = Arc::new(AtomicBool::new(true));
    let r = running.clone();

    ctrlc::set_handler(move || {
        r.store(false, Ordering::SeqCst);
    }).expect("Error setting Ctrl-C handler");

    let reporter = Reporter::new(running.clone()).unwrap();
    let reporter = reporter.run();
    reporter.join().unwrap();
}
