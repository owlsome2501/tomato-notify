use std::error::Error;
use std::fmt::Debug;
use std::io;
use std::time::{Duration, Instant};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{UnixListener, UnixStream};
use tokio::process::Command;
use tokio::signal;
use tokio::sync::{mpsc, watch};
use tokio::task::JoinHandle;
use tokio::time::{sleep, timeout};

static UNIX_ADDRESS: &'static str = "/tmp/tomato-notity-socket";
static BUSY_DURATION: u64 = 1;
static SHORT_BREAK_DURATION: u64 = 1;
static LONG_BREAK_DURATION: u64 = 1;
static NOTIFY_REMIND_DURATION: u64 = 1;

#[inline(always)]
fn min2sec(min: u64) -> u64 {
    min * 20
}

fn dura_sub(lhs: &Duration, rhs: &Duration) -> i32 {
    match lhs.checked_sub(*rhs) {
        Some(gt) => gt.as_secs() as i32,
        None => match rhs.checked_sub(*lhs) {
            Some(lt) => -(lt.as_secs() as i32),
            None => 0,
        },
    }
}

async fn sleep_min(min: u64) {
    sleep(Duration::from_secs(min2sec(min))).await;
}

type Quit = watch::Receiver<bool>;

fn prepare_quit_signal() -> (JoinHandle<()>, Quit) {
    let (tx, rx) = watch::channel(false);
    let handle = tokio::spawn(async move {
        signal::ctrl_c().await.unwrap();
        tx.send(true).unwrap();
    });
    (handle, rx)
}

#[derive(Debug)]
enum TomatoActions {
    Ready,
    Remind,
}

#[derive(Clone, Debug)]
enum TomatoStatus {
    Busy,
    ShortBreak,
    LongBreak,
}

#[derive(Clone, Debug)]
struct ClockInfo {
    tomato_status: TomatoStatus,
    next_status: TomatoStatus,
    need_notify: bool,
    last_checked: Instant,
}

struct TomatoClock {
    status_output: watch::Sender<ClockInfo>,
    action_input: mpsc::Receiver<TomatoActions>,
}

impl TomatoClock {
    fn new() -> (
        Self,
        mpsc::Sender<TomatoActions>,
        watch::Receiver<ClockInfo>,
    ) {
        let status = ClockInfo {
            tomato_status: TomatoStatus::Busy,
            next_status: TomatoStatus::ShortBreak,
            need_notify: false,
            last_checked: Instant::now(),
        };
        let (action_tx, action_rx) = mpsc::channel(8);
        let (status_tx, status_rx) = watch::channel(status);
        let tomato_clock = Self {
            status_output: status_tx,
            action_input: action_rx,
        };
        (tomato_clock, action_tx, status_rx)
    }

    // [Fix me] bad solution
    async fn _clear_action_input(&mut self) {
        let exhaust_receiver = async { while let Some(_) = self.action_input.recv().await {} };
        let _ = timeout(Duration::from_millis(100), exhaust_receiver).await;
    }

    async fn _wait_for_ready_action(&mut self) {
        loop {
            // action_input.recv return None when other senders were closed
            let action = self.action_input.recv().await.unwrap();
            if let TomatoActions::Ready = action {
                break;
            }
        }
    }

    async fn notify(&mut self) {
        self._clear_action_input().await;
        loop {
            let mut notify_status = self.status_output.borrow().clone();
            notify_status.need_notify = true;
            self.status_output.send(notify_status).unwrap();
            tokio::select! {
                _ = self._wait_for_ready_action() => {break;},
                _ = sleep_min(NOTIFY_REMIND_DURATION) => {},
            }
        }
    }

    async fn clock_logic(&mut self) {
        let mut manual_start = true;
        loop {
            for tomato in 0..4 {
                if manual_start {
                    manual_start = false;
                } else {
                    self.notify().await;
                }
                let status = ClockInfo {
                    tomato_status: TomatoStatus::Busy,
                    next_status: if tomato != 3 {
                        TomatoStatus::ShortBreak
                    } else {
                        TomatoStatus::LongBreak
                    },
                    need_notify: false,
                    last_checked: Instant::now(),
                };
                self.status_output.send(status).unwrap();
                sleep_min(BUSY_DURATION).await;

                if tomato != 3 {
                    self.notify().await;
                    let status = ClockInfo {
                        tomato_status: TomatoStatus::ShortBreak,
                        next_status: TomatoStatus::Busy,
                        need_notify: false,
                        last_checked: Instant::now(),
                    };
                    self.status_output.send(status).unwrap();
                    sleep_min(SHORT_BREAK_DURATION).await;
                } else {
                    self.notify().await;
                    let status = ClockInfo {
                        tomato_status: TomatoStatus::LongBreak,
                        next_status: TomatoStatus::Busy,
                        need_notify: false,
                        last_checked: Instant::now(),
                    };
                    self.status_output.send(status).unwrap();
                    sleep_min(LONG_BREAK_DURATION).await;
                }
            }
        }
    }

    async fn run(mut self, mut control: Quit) {
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

struct SocketRemote {
    listener: UnixListener,
    clock_status: watch::Receiver<ClockInfo>,
    action_sender: mpsc::Sender<TomatoActions>,
}

impl SocketRemote {
    fn new(
        clock_status: watch::Receiver<ClockInfo>,
        action_sender: mpsc::Sender<TomatoActions>,
    ) -> Result<Self, Box<dyn Error>> {
        let reporter = SocketRemote {
            listener: UnixListener::bind(UNIX_ADDRESS)?,
            clock_status,
            action_sender,
        };
        Ok(reporter)
    }

    fn api_get_info(&self) -> String {
        let clock_status = self.clock_status.borrow();
        let elapsed = clock_status.last_checked.elapsed();
        let due_duration = match clock_status.tomato_status {
            TomatoStatus::Busy => Duration::from_secs(min2sec(BUSY_DURATION)),
            TomatoStatus::ShortBreak => Duration::from_secs(min2sec(SHORT_BREAK_DURATION)),
            TomatoStatus::LongBreak => Duration::from_secs(min2sec(LONG_BREAK_DURATION)),
        };
        dura_sub(&due_duration, &elapsed).to_string()
    }

    async fn api_ready(&self) -> String {
        self.action_sender.send(TomatoActions::Ready).await.unwrap();
        "OK".to_string()
    }

    async fn api_remind(&self) -> String {
        self.action_sender
            .send(TomatoActions::Remind)
            .await
            .unwrap();
        "OK".to_string()
    }

    async fn send(&self, stream: &mut UnixStream, command: &str) -> std::io::Result<()> {
        let msg = match command {
            "GET INFO" => self.api_get_info(),
            "READY" => self.api_ready().await,
            "REMIND" => self.api_remind().await,
            _ => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "invalid command",
                ))
            }
        };
        match stream.write_all(&msg.into_bytes()).await {
            Ok(_n) => Ok(()),
            Err(e) => Err(e.into()),
        }
    }

    // client must close write stream after send the whole command
    async fn receive(&self, stream: &mut UnixStream) -> std::io::Result<String> {
        let mut dst: [u8; 32] = [0; 32];
        {
            let mut buf = &mut dst[..];
            loop {
                match stream.read_buf(&mut buf).await {
                    Ok(0) => {
                        break;
                    }
                    Ok(_n) => {
                        continue;
                    }
                    Err(e) => return Err(e.into()),
                }
            }
        }
        let l = match (&mut dst).iter().position(|&b| b == ('\n' as u8)) {
            Some(l) if l == 0 => {
                return Err(io::Error::new(io::ErrorKind::InvalidData, "empty command"))
            }
            Some(l) => l,
            None => return Err(io::Error::new(io::ErrorKind::InvalidData, r"no \n")),
        };
        let buf = &dst[..l];
        let buf = match std::str::from_utf8(&buf) {
            Ok(v) => v,
            Err(_e) => return Err(io::Error::new(io::ErrorKind::InvalidData, "not utf-8")),
        };
        Ok(buf.to_string())
    }

    async fn handle_stream(&self, mut stream: UnixStream) -> Result<(), Box<dyn Error>> {
        let command = match self.receive(&mut stream).await {
            Ok(command) => command,
            Err(e) => {
                return Err(e.into());
            }
        };
        match self.send(&mut stream, &command).await {
            Ok(_) => Ok(()),
            Err(e) => Err(e.into()),
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

impl Drop for SocketRemote {
    fn drop(&mut self) {
        std::fs::remove_file(UNIX_ADDRESS).unwrap();
    }
}

struct NotifierRemote {
    backend: String,
    clock_status: watch::Receiver<ClockInfo>,
    action_sender: mpsc::Sender<TomatoActions>,
}

impl NotifierRemote {
    fn new(
        clock_status: watch::Receiver<ClockInfo>,
        action_sender: mpsc::Sender<TomatoActions>,
    ) -> Self {
        let backend = "dunstify";
        NotifierRemote {
            backend: backend.to_string(),
            clock_status,
            action_sender,
        }
    }

    async fn run(mut self, mut control: Quit) {
        tokio::select! {
            _ = self.tomato_notify() => {},
            res = control.changed() => {
                match res {
                    Ok(_) => {
                        println!("NotifierRemote exit with {:?}", res);
                        println!("control = {}", *control.borrow());
                        if *control.borrow() {
                            return;
                        }
                    },
                    Err(e) => {
                        eprintln!("NotifierRemote quit control: {}", e);
                        return;
                    }
                }
            }
        }
    }

    async fn tomato_notify(&mut self) {
        while self.clock_status.changed().await.is_ok() {
            if !self.clock_status.borrow().need_notify {
                continue;
            }
            let msg = match self.clock_status.borrow().next_status {
                TomatoStatus::Busy => "Start work",
                TomatoStatus::ShortBreak => "Shor break",
                TomatoStatus::LongBreak => "Long break",
            };
            let actions = [("ok", "Ok"), ("remind", "Remind me later")];
            let selection = self._notify(msg, &actions).await;
            match selection {
                Ok(Some(selection)) if selection == "ok" => {
                    self.action_sender.send(TomatoActions::Ready).await.unwrap();
                }
                _ => {
                    self.action_sender
                        .send(TomatoActions::Remind)
                        .await
                        .unwrap();
                }
            }
        }
    }

    async fn _notify(&self, msg: &str, actions: &[(&str, &str)]) -> Result<Option<String>, ()> {
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

#[tokio::main]
async fn main() {
    let (quit_handle, quit_signal) = prepare_quit_signal();

    let (clock, action_sender, clock_status) = TomatoClock::new();
    let socket_remote = SocketRemote::new(clock_status.clone(), action_sender.clone()).unwrap();
    let notifier_remote = NotifierRemote::new(clock_status.clone(), action_sender.clone());

    let clock_quit_signal = quit_signal.clone();
    let clock_handle = tokio::spawn(async move { clock.run(clock_quit_signal).await });

    let socket_remote_quit_signal = quit_signal.clone();
    let socket_remote_handle =
        tokio::spawn(async move { socket_remote.run(socket_remote_quit_signal).await });

    let notifier_remote_quit_signal = quit_signal.clone();
    let notifier_remote_handle =
        tokio::spawn(async move { notifier_remote.run(notifier_remote_quit_signal).await });

    quit_handle.await.unwrap();
    clock_handle.await.unwrap();
    socket_remote_handle.await.unwrap();
    notifier_remote_handle.await.unwrap();
}
