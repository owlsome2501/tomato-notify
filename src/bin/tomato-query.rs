use std::io::prelude::*;
use std::os::unix::net::UnixStream;

static UNIX_ADDRESS: &'static str = "/tmp/tomato-notity-socket";

fn main() {
    let mut stream = UnixStream::connect(UNIX_ADDRESS).unwrap();
    let mut response = String::new();
    stream.read_to_string(&mut response).unwrap();
    println!("{}", response);
}
