use clap::{App, SubCommand};
// use std::env;
use std::io::prelude::*;
use std::os::unix::net::UnixStream;

static UNIX_ADDRESS: &'static str = "/tmp/tomato-notity-socket";
// static USAGE: &'static str = "tomato-query [ --get-info | --ready | --remind ]";

fn main() {
    let matches = App::new("tomatoctl")
        .version("1.0.0")
        .author("Richard Yao <2501.owlsome@gmail.com>")
        .about("a tomato-notify client")
        .subcommand(
            SubCommand::with_name("status")
                .about("get clock status")
                .arg_from_usage("--polybar 'use polybar style output'")
                .arg_from_usage("--style=[FILE] 'custom polybar output style [not implemented]'"),
        )
        .subcommand(SubCommand::with_name("ready").about("send ready action"))
        .subcommand(SubCommand::with_name("remind").about("send remind action"))
        .get_matches();
    match matches.subcommand() {
        ("status", Some(sub)) => {
            let out = tomato("GET INFO".to_string());
            output(out, sub.is_present("polybar"), sub.value_of("style"));
        }
        ("ready", Some(_sub)) => {
            let _ = tomato("READY".to_string());
        }
        ("remind", Some(_sub)) => {
            let _ = tomato("REMIND".to_string());
        }
        _ => {
            let out = tomato("GET INFO".to_string());
            output(out, false, None);
        }
    }
}

fn tomato(mut command: String) -> String {
    command.push_str("\n");

    let mut stream = UnixStream::connect(UNIX_ADDRESS).unwrap();
    let mut response = String::new();
    stream.write_all(command.as_bytes()).unwrap();
    // stream.write_all("GET ".as_bytes()).unwrap();
    // std::thread::sleep(std::time::Duration::from_millis(800));
    // stream.write_all("INFO\n".as_bytes()).unwrap();
    stream.shutdown(std::net::Shutdown::Write).unwrap();
    stream.read_to_string(&mut response).unwrap();
    response
}

fn output(out: String, is_polybar: bool, _style: Option<&str>) {
    if is_polybar {
        let mut secs: i32 = out.parse().unwrap();
        let mut minus = false;
        if secs < 0 {
            secs = -secs;
            minus = true;
        }
        let mins = secs / 60;
        secs = secs % 60;
        println!("{}{:02}:{:02}", if minus { "-" } else { " " }, mins, secs);
    // println!("out = {}, {:?}, {:?}", out, is_polybar, style);
    } else {
        println!("{}", out);
    }
}
