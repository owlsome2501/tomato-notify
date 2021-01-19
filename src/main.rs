use std::io;
use std::cell::RefCell;
use std::error::Error;
use std::time::Duration;
use tokio::process::Command;
use tokio::net::{UnixListener, UnixStream};
use tokio::signal;
use tokio::sync::watch;
use tokio::time::timeout;
use tokio::task::JoinHandle;

static UNIX_ADDRESS: &'static str = "/tmp/tomato-notity-socket";

type Quit = watch::Receiver<bool>;

struct Reporter {
    listener: UnixListener,
    control: Quit,
}

impl Reporter {
    fn new(control: Quit) -> Result<Reporter, Box<dyn Error>> {
        let reporter = Reporter {
            listener: UnixListener::bind(UNIX_ADDRESS)?,
            control,
        };
        Ok(reporter)
    }

    fn report(stream: &UnixStream) -> std::io::Result<()> {
        match stream.try_write(b"hello world") {
            Ok(_n) => Ok(()),
            Err(e) => Err(e.into()),
        }
    }

    async fn handle_stream(stream: UnixStream) -> Result<(), Box<dyn Error>> {
        loop {
            stream.writable().await?;
            match Reporter::report(&stream) {
                Ok(_) => break Ok(()),
                Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => {
                    println!("WouldBlock");
                    continue;
                }
                Err(e) => {
                    return Err(e.into());
                }
            }
        }
    }

    async fn run(mut self) {
        loop {
            tokio::select! {
                res = self.listener.accept() => {
                    match res {
                        Ok((stream, _addr)) => {
                            match timeout(Duration::from_millis(1000),
                                Reporter::handle_stream(stream)).await {
                                Err(_) => eprintln!("process timeout"),
                                Ok(Err(e)) => eprintln!("{}", e),
                                Ok(Ok(_)) => {},
                            }
                        },
                        Err(e) => {
                            eprintln!("accept stream failed: {}", e);
                            break;
                        },
                    }
                },
                res = self.control.changed() => {
                    match res {
                        Ok(_) => {
                            println!("exit with {:?}", res);
                            println!("control = {}", *self.control.borrow());
                            if *self.control.borrow() {
                                break;
                            }
                        },
                        Err(e) => {
                            eprintln!("quit control: {}", e);
                            break;
                        }
                    }
                }
            }
        }
    }
}

impl Drop for Reporter {
    fn drop(&mut self) {
       std::fs::remove_file(UNIX_ADDRESS).unwrap();
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

    async fn notify(&self, msg: &str, actions: &[(&str, &str)]) -> Result<String, ()> {
        let actions: Vec<String> = actions
            .iter()
            .map(|(action, label)| format!("--action={},{}", action, label))
            .collect();
        let mut args: Vec<String> = Vec::new();
        args.extend(actions);
        args.push(msg.to_string());
        let output = self.backend.borrow_mut().args(&args).output().await.unwrap();
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

fn prepare_quit_signal() -> (JoinHandle<()>, Quit) {
    let (tx, rx) = watch::channel(false);
    let handle = tokio::spawn(async move {
        signal::ctrl_c().await.unwrap();
        println!("get ctrl_c");
        tx.send(true).unwrap();
    });
    (handle, rx)
}

#[tokio::main]
async fn main() {
    let notifier = Notifier::new();
    if let Ok(selection) = notifier.notify("help me", &[("ok","Ok"),("like","Like")]).await {
        println!("{}", selection);
    };

    // let (quit_handle, quit_signal) = prepare_quit_signal();
    // let reporter = Reporter::new(quit_signal.clone()).unwrap();
    // let repoter_handle = tokio::spawn(async move {
    //     reporter.run().await
    // });
    // quit_handle.await.unwrap();
    // repoter_handle.await.unwrap();
}
