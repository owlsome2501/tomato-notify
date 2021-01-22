use std::io::prelude::*;
use std::os::unix::net::UnixStream;

static UNIX_ADDRESS: &'static str = "/tmp/tomato-notity-socket";

fn main() {
    let mut stream = UnixStream::connect(UNIX_ADDRESS).unwrap();
    let mut response = String::new();
    stream.write_all("GET INFO\n".as_bytes()).unwrap();
    // stream.write_all("GET ".as_bytes()).unwrap();
    // std::thread::sleep(std::time::Duration::from_millis(800));
    // stream.write_all("INFO\n".as_bytes()).unwrap();
    stream.shutdown(std::net::Shutdown::Write).unwrap();
    stream.read_to_string(&mut response).unwrap();
    println!("{}", response);
}
