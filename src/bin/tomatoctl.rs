use clap::{App, SubCommand};
// use std::env;
use std::io;
use std::io::prelude::*;
use std::os::unix::net::UnixStream;

static UNIX_ADDRESS: &'static str = "/tmp/tomato-notity-socket";

fn main() {
    let matches = App::new("tomatoctl")
        .version("1.1.0")
        .author("Richard Yao <2501.owlsome@gmail.com>")
        .about("a tomato-notify client")
        .subcommand(
            SubCommand::with_name("status")
                .about("get clock status")
                .arg_from_usage("--polybar 'use polybar style output'")
                .arg_from_usage("--stay 'stay foreground and undate per second'")
                .arg_from_usage("--style=[FILE] 'custom polybar output style [not implemented]'"),
        )
        .subcommand(SubCommand::with_name("ready").about("send ready action"))
        .subcommand(SubCommand::with_name("remind").about("send remind action"))
        .get_matches();
    match matches.subcommand() {
        ("status", Some(sub)) => {
            status_output(
                sub.is_present("polybar"),
                sub.is_present("stay"),
                sub.value_of("style"),
            );
        }
        ("ready", Some(_sub)) => {
            let _ = tomato("READY".to_string());
        }
        ("remind", Some(_sub)) => {
            let _ = tomato("REMIND".to_string());
        }
        _ => {
            status_output(false, false, None);
        }
    }
}

fn tomato(mut command: String) -> io::Result<String> {
    command.push_str("\n");

    let mut stream = UnixStream::connect(UNIX_ADDRESS)?;
    let mut response = String::new();
    stream.write_all(command.as_bytes())?;
    // sending parts in different time also useful
    //
    //     stream.write_all("GET ".as_bytes()).unwrap();
    //     std::thread::sleep(std::time::Duration::from_millis(800));
    //     stream.write_all("INFO\n".as_bytes()).unwrap();
    //
    stream.shutdown(std::net::Shutdown::Write)?;
    stream.read_to_string(&mut response)?;
    Ok(response)
}

struct PolybarStyle {}

fn status_output(is_polybar: bool, stay: bool, _style_file: Option<&str>) {
    let style = PolybarStyle {};
    let data = tomato("GET INFO".to_string());
    match data {
        Ok(data) => println!("{}", format(data, is_polybar, &style)),
        Err(_) => {
            println!("");
            return;
        }
    }
    while stay {
        std::thread::sleep(std::time::Duration::from_secs(1));
        let data = tomato("GET INFO".to_string());
        match data {
            Ok(data) => println!("{}", format(data, is_polybar, &style)),
            Err(_) => {
                println!("");
                return;
            }
        }
    }
}

fn format(data: String, is_polybar: bool, _style: &PolybarStyle) -> String {
    if is_polybar {
        let mut secs = match data.parse::<i32>() {
            Ok(secs) => secs,
            Err(_) => return "".to_string(),
        };
        let mut minus = false;
        if secs < 0 {
            secs = -secs;
            minus = true;
        }
        let mins = secs / 60;
        secs = secs % 60;
        format!("{}{:02}:{:02}", if minus { "-" } else { " " }, mins, secs)
    } else {
        format!("{}", data)
    }
}
