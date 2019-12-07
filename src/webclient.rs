use std::str;
use std::sync::mpsc::Sender;
use std::thread;
use std::time::Duration;

use curl::easy::Easy;
use serde::{Deserialize, Serialize};
use url::form_urlencoded;

#[derive(Debug)]
pub struct It {
    pub host: String,
    pub fd: i8,
    pub line: String,
}

#[derive(Debug)]
pub struct Opts {
    pub host: String,
    pub port: u32,
    pub command: String,
    pub arguments: Vec<String>,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct Result {
    pub stdout: String,
    pub stderr: String,

    #[serde(default)]
    pub exit_code: i32,

    #[serde(default)]
    pub error: String,
}

impl Opts {
    pub fn new(host: String, port: u32, command: String, arguments: Vec<String>) -> Opts {
        Opts {
            host: host,
            port: port,
            command: command,
            arguments: arguments,
        }
    }

    fn build_url(self: &Self, chunked: bool) -> String {
        let mut param_builder = form_urlencoded::Serializer::new(String::new());
        if chunked {
            param_builder.append_pair("chunked", "1");
        }
        if !self.arguments.is_empty() {
            param_builder.append_pair("argument", self.arguments.join(" ").as_str());
        }
        let param = param_builder.finish();
        format!(
            "http://{}:{}/command/{}?{}",
            self.host, self.port, self.command, param
        )
    }
}

fn parse_chunk(s: &str) -> (i8, &str) {
    let mut v = s.splitn(2, "> ");
    let fd: i8 = v.next().unwrap().parse().unwrap();
    let line = v.next().unwrap();
    (fd, line)
}

pub fn run_command(opts: Opts) -> Result {
    let mut dst = Vec::new();
    let mut easy = Easy::new();
    easy.connect_timeout(Duration::new(3, 0)).unwrap();
    easy.url(opts.build_url(false).as_str()).unwrap();
    {
        let mut transfer = easy.transfer();
        transfer
            .write_function(|data| {
                dst.extend_from_slice(data);
                Ok(data.len())
            })
            .unwrap();
        transfer.perform().unwrap();
    }
    let body = String::from_utf8(dst).unwrap();
    serde_json::from_str(body.as_str()).unwrap()
}

pub fn run_realtime(opts: Opts, tx: Sender<It>) {
    let mut easy = Easy::new();
    easy.connect_timeout(Duration::new(3, 0)).unwrap();
    easy.url(opts.build_url(true).as_str()).unwrap();
    let mut transfer = easy.transfer();
    transfer
        .write_function(|data| {
            let v = str::from_utf8(data).unwrap();
            let (fd, line) = parse_chunk(v);
            tx.send(It {
                host: opts.host.clone(),
                fd: fd,
                line: line.to_string(),
            })
            .unwrap();
            Ok(data.len())
        })
        .unwrap();
    transfer.perform().unwrap_or_else(|e| {
        tx.send(It {
            host: opts.host.clone(),
            fd: 0,
            line: format!("{{\"error\": \"{}\"}}", e),
        })
        .unwrap();
    })
}

pub fn run_parallel(opts: Opts, tx: Sender<It>) {
    let hosts: Vec<&str> = opts.host.split(",").collect();
    let mut children = Vec::new();
    for host in hosts {
        let opts = Opts {
            host: host.to_string(),
            port: opts.port,
            command: opts.command.clone(),
            arguments: opts.arguments.clone(),
        };
        let tx = tx.clone();
        let child = thread::spawn(move || run_realtime(opts, tx));
        children.push(child);
    }
    for child in children {
        child.join().unwrap();
    }
}
