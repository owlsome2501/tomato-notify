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
    min * 5
}

fn dura_sub(lhs: &Duration, rhs: &Duration) -> i32 {
    match lhs.checked_sub(*rhs) {
        Some(gt) => gt.as_secs() as i32,
        None => match rhs.checked_sub(*lhs) {
            Some(lt) => - (lt.as_secs() as i32),
            None => 0,
        }
    }
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

    async fn clock_logic(&self) {
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

    async fn run(self, mut control: Quit) {
        tokio::select! {
            // the whole clock_logic process can be intercepted
            _ = self.clock_logic() => {},
            res = control.changed() => {
                match res {
                    Ok(_) => {
                        println!("clock exit with {:?}", res);
                        println!("control = {}", *control.borrow());
                        if *control.borrow() {
                            return;
                        }
                    },
                    Err(e) => {
                        eprintln!("clock quit control: {}", e);
                        return;
                    }
                }
            }
        }
    }
}

type Quit = watch::Receiver<bool>;

struct Remote {
    listener: UnixListener,
    clock_status: Arc<Mutex<ClockInfo>>,
}

impl Remote {
    fn new(clock_status: Arc<Mutex<ClockInfo>>) -> Result<Self, Box<dyn Error>> {
        let reporter = Remote {
            listener: UnixListener::bind(UNIX_ADDRESS)?,
            clock_status,
        };
        Ok(reporter)
    }

    fn api_get_info(&self) -> String {
            let clock_status = self.clock_status.lock().unwrap();
            let elapsed = clock_status.last_checked.elapsed();
            let due_duration = match clock_status.tomato_status {
                TomatoStatus::Busy => {
                    Duration::from_secs(min2sec(BUSY_DURATION))
                },
                TomatoStatus::ShortBreak => {
                    Duration::from_secs(min2sec(SHORT_BREAK_DURATION))
                },
                TomatoStatus::LongBreak => {
                    Duration::from_secs(min2sec(LONG_BREAK_DURATION))
                },
            };
            dura_sub(&due_duration, &elapsed).to_string()
    }

    // may can't send all data
    fn send(&self, stream: &UnixStream, command: &str) -> std::io::Result<()> {
        let msg = match command {
            "GET INFO" => self.api_get_info(),
            _ => return Err(io::Error::new(io::ErrorKind::InvalidData, "invalid command")),
        };
        match stream.try_write(&msg.into_bytes()) {
            Ok(_n) => Ok(()),
            Err(e) => Err(e.into()),
        }
    }

    // may can't receive all data
    fn receive(&self, stream: &UnixStream) -> std::io::Result<String> {
        let mut buf = [0; 32];
        match stream.try_read(&mut buf) {
            Ok(0) => return Err(io::Error::new(io::ErrorKind::WouldBlock, "empty read")),
            Ok(_n) => {},
            Err(e) => return Err(e.into()),
        }
        let buf = match std::str::from_utf8(&buf) {
            Ok(v) => v,
            Err(_e) => return Err(io::Error::new(io::ErrorKind::InvalidData, "not utf-8")),
        };
        let l = match buf.find("\n") {
            Some(l) if l == 0 => return Err(io::Error::new(io::ErrorKind::InvalidData, "empty command")),
            Some(l) => l,
            None => return Err(io::Error::new(io::ErrorKind::InvalidData, r"no \n")),
        };
        Ok(buf[0..l].to_string())
    }

    async fn handle_stream(&self, stream: UnixStream) -> Result<(), Box<dyn Error>> {
        let command = loop {
            stream.readable().await?;
            match self.receive(&stream) {
                Ok(command) => break command,
                Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => {
                    continue;
                }
                Err(e) => {
                    return Err(e.into());
                }
            }
        };
        loop {
            stream.writable().await?;
            match self.send(&stream, &command) {
                Ok(_) => return Ok(()),
                Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => {
                    continue;
                }
                Err(e) => {
                    return Err(e.into());
                }
            }
        }
    }

    async fn run(self, mut control: Quit) {
        loop {
            tokio::select! {
                res = self.listener.accept() => {
                    match res {
                        Ok((stream, _addr)) => {
                            match timeout(Duration::from_millis(1000),
                                self.handle_stream(stream)).await {
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
                res = control.changed() => {
                    match res {
                        Ok(_) => {
                            println!("remote exit with {:?}", res);
                            println!("control = {}", *control.borrow());
                            if *control.borrow() {
                                break;
                            }
                        },
                        Err(e) => {
                            eprintln!("remote quit control: {}", e);
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
        let mut command = Command::new(&self.backend);
        let command = command.kill_on_drop(true).args(&args);
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
    let (quit_handle, quit_signal) = prepare_quit_signal();

    let clock = TomatoClock::new();
    let remote = Remote::new(clock.get_info()).unwrap();

    let clock_quit_signal = quit_signal.clone();
    let clock_handle = tokio::spawn(async move {
        clock.run(clock_quit_signal).await;
    });

    let remote_quit_signal = quit_signal.clone();
    let remote_handle = tokio::spawn(async move {
        remote.run(remote_quit_signal).await
    });

    quit_handle.await.unwrap();
    clock_handle.await.unwrap();
    remote_handle.await.unwrap();
}
