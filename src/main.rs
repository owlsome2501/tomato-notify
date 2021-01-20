use std::error::Error;
use std::io;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::net::{UnixListener, UnixStream};
use tokio::process::Command;
use tokio::signal;
use tokio::sync::watch;
use tokio::task::JoinHandle;
use tokio::time::{sleep, timeout};

static UNIX_ADDRESS: &'static str = "/tmp/tomato-notity-socket";
static BUSY_DURATION: u64 = 1;
static SHORT_BREAK_DURATION: u64 = 1;
static LONG_BREAK_DURATION: u64 = 1;
static NOTIFY_REMIND_DURATION: u64 = 1;

#[inline(always)]
fn min2sec(min: u64) -> u64 {
    min * 10
}

async fn sleep_min(min: u64) {
    sleep(Duration::from_secs(min2sec(min))).await;
}

enum TomatoStatus {
    Busy,
    ShortBreak,
    LongBreak,
}

struct ClockInfo {
    tomato_status: TomatoStatus,
    last_checked: Instant,
}

struct TomatoClock {
    status: Arc<Mutex<ClockInfo>>,
    notifier: Notifier,
}

impl TomatoClock {
    fn new() -> Self {
        Self {
            status: Arc::new(Mutex::new(ClockInfo {
                tomato_status: TomatoStatus::Busy,
                last_checked: Instant::now(),
            })),
            notifier: Notifier::new(),
        }
    }

    fn get_info(&self) -> Arc<Mutex<ClockInfo>> {
        Arc::clone(&self.status)
    }

    async fn notify(&self, msg: &str) -> bool {
        let actions = [("ok", "Ok"), ("remind", "Remind me later")];
        let selection = self.notifier.notify(msg, &actions).await;
        match selection {
            Ok(Some(selection)) if selection == "ok" => true,
            _ => false,
        }
    }

    async fn run(self) {
        let mut manual_start = true;
        loop {
            for tomato in 0..4 {
                if manual_start {
                    manual_start = false;
                } else {
                    while !self.notify("start work").await {
                        sleep_min(NOTIFY_REMIND_DURATION).await;
                    }
                }
                {
                    let mut status = self.status.lock().unwrap();
                    status.last_checked = Instant::now();
                    status.tomato_status = TomatoStatus::Busy;
                }
                sleep_min(BUSY_DURATION).await;

                if tomato != 3 {
                    while !self.notify("short break").await {
                        sleep_min(NOTIFY_REMIND_DURATION).await;
                    }
                    {
                        let mut status = self.status.lock().unwrap();
                        status.last_checked = Instant::now();
                        status.tomato_status = TomatoStatus::ShortBreak;
                    }
                    sleep_min(SHORT_BREAK_DURATION).await;
                } else {
                    while !self.notify("long break").await {
                        sleep_min(NOTIFY_REMIND_DURATION).await;
                    }
                    {
                        let mut status = self.status.lock().unwrap();
                        status.last_checked = Instant::now();
                        status.tomato_status = TomatoStatus::LongBreak;
                    }
                    sleep_min(LONG_BREAK_DURATION).await;
                }
            }
        }
    }
}

type Quit = watch::Receiver<bool>;

struct Remote {
    listener: UnixListener,
    control: Quit,
}

impl Remote {
    fn new(control: Quit) -> Result<Self, Box<dyn Error>> {
        let reporter = Remote {
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
            match Remote::report(&stream) {
                Ok(_) => break Ok(()),
                Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => {
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
                                Remote::handle_stream(stream)).await {
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

impl Drop for Remote {
    fn drop(&mut self) {
        std::fs::remove_file(UNIX_ADDRESS).unwrap();
    }
}

struct Notifier {
    backend: String,
}

impl Notifier {
    fn new() -> Self {
        let backend = "dunstify";
        Notifier {
            backend: backend.to_string(),
        }
    }

    async fn notify(&self, msg: &str, actions: &[(&str, &str)]) -> Result<Option<String>, ()> {
        let actions: Vec<String> = actions
            .iter()
            .map(|(action, label)| format!("--action={},{}", action, label))
            .collect();
        let mut args: Vec<String> = Vec::new();
        args.extend(actions);
        args.push(msg.to_string());
        let mut command = Command::new("dunstify");
        let command = command.args(&args);
        let output = command.output().await.unwrap();
        if output.status.success() {
            let output = String::from_utf8(output.stdout).unwrap().trim().to_string();
            if output == "2" {
                Ok(None)
            } else {
                Ok(Some(output))
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
        tx.send(true).unwrap();
    });
    (handle, rx)
}

#[tokio::main]
async fn main() {
    let clock = TomatoClock::new();
    tokio::spawn(async move {
        clock.run().await;
    })
    .await
    .unwrap();

    // let (quit_handle, quit_signal) = prepare_quit_signal();
    // let Remote = Remote::new(quit_signal.clone()).unwrap();
    // let repoter_handle = tokio::spawn(async move {
    //     Remote.run().await
    // });
    // quit_handle.await.unwrap();
    // repoter_handle.await.unwrap();
}
