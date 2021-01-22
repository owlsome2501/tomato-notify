use std::env;
use std::io::prelude::*;
use std::os::unix::net::UnixStream;

static UNIX_ADDRESS: &'static str = "/tmp/tomato-notity-socket";
static USAGE: &'static str = "tomato-query [ --get-info | --ready | --remind ]";

fn main() {
    let args: Vec<String> = env::args().collect();
    if args.len() > 2 {
        panic!(USAGE);
    }
    let command = match args.get(1) {
        Some(command) => match command.as_str() {
            "--get-info" => "GET INFO",
            "--ready" => "READY",
            "--remind" => "REMIND",
            _ => panic!(USAGE),
        },
        None => "GET INFO",
    };
    let mut command = command.to_string();
    command.push_str("\n");

    let mut stream = UnixStream::connect(UNIX_ADDRESS).unwrap();
    let mut response = String::new();
    stream.write_all(command.as_bytes()).unwrap();
    // stream.write_all("GET ".as_bytes()).unwrap();
    // std::thread::sleep(std::time::Duration::from_millis(800));
    // stream.write_all("INFO\n".as_bytes()).unwrap();
    stream.shutdown(std::net::Shutdown::Write).unwrap();
    stream.read_to_string(&mut response).unwrap();
    println!("{}", response);
}
