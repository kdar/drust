#![feature(unboxed_closures)]

extern crate pbr;
extern crate hyper;
#[macro_use]
extern crate clap;
#[macro_use]
extern crate log;
extern crate simplelog;
extern crate url;
extern crate cue;
#[macro_use]
extern crate lazy_static;

use simplelog::{TermLogger, LogLevelFilter};
use hyper::client::{Client, IntoUrl};
use hyper::header::{AcceptRanges, RangeUnit, ContentLength, Range, Connection};
use pbr::{ProgressBar, Units};
use std::fs::File;
use std::io::{Write, Read, SeekFrom, Seek};
use std::path::Path;
use clap::{Arg, App};
use std::process::exit;
use url::Url;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use std::collections::HashMap;

const BYTE: u64 = 1 << 0;
const KIBYTE: u64 = 1 << 10;
const MIBYTE: u64 = 1 << 20;
const GIBYTE: u64 = 1 << 30;
const TIBYTE: u64 = 1 << 40;
const PIBYTE: u64 = 1 << 50;
const EIBYTE: u64 = 1 << 60;

const IBYTE: u64 = 1;
const KBYTE: u64 = IBYTE * 1000;
const MBYTE: u64 = KBYTE * 1000;
const GBYTE: u64 = MBYTE * 1000;
const TBYTE: u64 = GBYTE * 1000;
const PBYTE: u64 = TBYTE * 1000;
const EBYTE: u64 = PBYTE * 1000;

lazy_static! {
  static ref BYTES_SIZE_TABLE: HashMap<&'static str, u64> = {
    let mut m = HashMap::new();
    m.insert("b", BYTE);
  	m.insert("kib", KIBYTE);
  	m.insert("kb", KBYTE);
  	m.insert("mib", MIBYTE);
  	m.insert("mb", MBYTE);
  	m.insert("gib", GIBYTE);
  	m.insert("gb", GBYTE);
  	m.insert("tib", TIBYTE);
  	m.insert("tb", TBYTE);
  	m.insert("pib", PIBYTE);
  	m.insert("pb", PBYTE);
  	m.insert("eib", EIBYTE);
  	m.insert("eb", EBYTE);
  	// Without suffix
  	m.insert("", BYTE);
  	m.insert("ki", KIBYTE);
  	m.insert("k", KBYTE);
  	m.insert("mi", MIBYTE);
  	m.insert("m", MBYTE);
  	m.insert("gi", GIBYTE);
  	m.insert("g", GBYTE);
  	m.insert("ti", TIBYTE);
  	m.insert("t", TBYTE);
  	m.insert("pi", PIBYTE);
  	m.insert("p", PBYTE);
  	m.insert("ei", EIBYTE);
  	m.insert("e", EBYTE);
    m
  };
}

macro_rules! try_s {
  ($expr:expr) => (match $expr {
    std::result::Result::Ok(val) => val,
    std::result::Result::Err(err) => {
      return std::result::Result::Err(err.to_string());
    }
  })
}

fn parse_bytes(s: &str) -> Result<u64, String> {
  let num = s.chars().take_while(|&ch| ch.is_numeric() || ch == '.');
  let rest = s.chars().skip_while(|&ch| ch.is_numeric() || ch == '.' || ch == ' ');
  // for ch in iter {
  //   if !ch.is_numeric() || ch == '.' {
  //     break;
  //   }
  // }

  let mut f = match num.collect::<String>().parse::<f64>() {
    Ok(v) => v,
    Err(e) => return Err(e.to_string()),
  };

  let rest = rest.collect::<String>().to_lowercase();
  if let Some(&b) = BYTES_SIZE_TABLE.get(rest.as_str()) {
    f *= b as f64;
    let f = f as u64;
    if f >= std::u64::MAX {
      return Err(format!("too large: {}", s));
    }

    return Ok(f);
  }

  Err(format!("unhandled size name: {}", rest))
}

#[test]
fn test_parse_bytes() {
  assert_eq!(parse_bytes("42"), Ok(42));
  assert_eq!(parse_bytes("42MB"), Ok(42000000));
  assert_eq!(parse_bytes("42MiB"), Ok(44040192));
  assert_eq!(parse_bytes("42mb"), Ok(42000000));
  assert_eq!(parse_bytes("42mib"), Ok(44040192));
  assert_eq!(parse_bytes("42MIB"), Ok(44040192));
  assert_eq!(parse_bytes("42 MB"), Ok(42000000));
  assert_eq!(parse_bytes("42 MiB"), Ok(44040192));
  assert_eq!(parse_bytes("42 mb"), Ok(42000000));
  assert_eq!(parse_bytes("42 mib"), Ok(44040192));
  assert_eq!(parse_bytes("42 MIB"), Ok(44040192));
  assert_eq!(parse_bytes("42.5MB"), Ok(42500000));
  assert_eq!(parse_bytes("42.5MiB"), Ok(44564480));
  assert_eq!(parse_bytes("42.5 MB"), Ok(42500000));
  assert_eq!(parse_bytes("42.5 MiB"), Ok(44564480));
  // No need to say B
  assert_eq!(parse_bytes("42M"), Ok(42000000));
  assert_eq!(parse_bytes("42Mi"), Ok(44040192));
  assert_eq!(parse_bytes("42m"), Ok(42000000));
  assert_eq!(parse_bytes("42mi"), Ok(44040192));
  assert_eq!(parse_bytes("42MI"), Ok(44040192));
  assert_eq!(parse_bytes("42 M"), Ok(42000000));
  assert_eq!(parse_bytes("42 Mi"), Ok(44040192));
  assert_eq!(parse_bytes("42 m"), Ok(42000000));
  assert_eq!(parse_bytes("42 mi"), Ok(44040192));
  assert_eq!(parse_bytes("42 MI"), Ok(44040192));
  assert_eq!(parse_bytes("42.5M"), Ok(42500000));
  assert_eq!(parse_bytes("42.5Mi"), Ok(44564480));
  assert_eq!(parse_bytes("42.5 M"), Ok(42500000));
  assert_eq!(parse_bytes("42.5 Mi"), Ok(44564480));
  // Large testing, breaks when too much larger than
  // this.
  assert_eq!(parse_bytes("12.5 EB"), Ok((12.5 * EBYTE as f64) as u64));
  assert_eq!(parse_bytes("12.5 E"), Ok((12.5 * EBYTE as f64) as u64));
  assert_eq!(parse_bytes("12.5 EiB"), Ok((12.5 * EIBYTE as f64) as u64));
}

fn check<U: IntoUrl>(url: U) -> Result<u64, String> {
  let client = Client::new();
  let res = try_s!(client.get(url)
    .send());

  // println!("{:?}", res.headers);

  if !res.headers.has::<AcceptRanges>() ||
     !res.headers.get::<AcceptRanges>().unwrap().0.contains(&RangeUnit::Bytes) {
    return Err("Server does not support ranges.".to_owned());
  }

  if !res.headers.has::<ContentLength>() || res.headers.get::<ContentLength>().unwrap().0 == 0 {
    return Err("Could not get the content length.".to_owned());
  }

  Ok(res.headers.get::<ContentLength>().unwrap().0)
}

struct Download {
  url: String,
  output: String,
  size: u64,
  chunk_size: u64,
  max_threads: usize,
  on_update: Box<Fn(usize) + Sync>,
}

impl Download {
  fn run(&self) -> Result<(), String> {
    let mut f = Mutex::new(File::create(self.output.clone()).unwrap());

    let chunk_count = (self.size as f64 / self.chunk_size as f64).round() as usize;
    info!("chunk_count: {}", chunk_count);

    let mut work: Vec<(u64, u64)> = vec![];
    if self.chunk_size >= self.size {
      work.push((0, self.size));
    } else {
      for i in 0..chunk_count {
        let offset = i as u64 * self.chunk_size;
        if i == chunk_count - 1 {
          work.push((offset, self.size));
        } else {
          work.push((offset, offset + self.chunk_size));
        }
      }
    }

    let mut count = 0;
    cue::pipeline("download",
                  self.max_threads,
                  work.iter(),
                  |item| {
                    // println!("{}MB - {}MB", item.0 / 1024 / 1024, item.1 / 1024 / 1024);
                    self.get_part(item.0, item.1)
                  },
                  |r| {
      count += 1;
      let data = match r {
        Ok(d) => d,
        Err(e) => {
          error!("{}", e);
          return;
        }
      };
      // println!("count: {}. data: {}", count, data.1.len());
      let mut f = f.lock().unwrap();
      f.seek(SeekFrom::Start(data.0)).unwrap();
      f.write(data.1.as_slice()).unwrap();
    });

    Ok(())
  }

  fn get_part(&self, start: u64, end: u64) -> Result<(u64, Vec<u8>), String> {
    let mut start_next = start;
    let mut buf: Vec<u8> = vec![0; (end - start) as usize];
    let mut len = 0;

    let mut sleep_on_error = 1000;
    let sleep_max = 10000;

    'outer: loop {
      let client = Client::new();
      let req = client.get(&self.url.clone());
      let mut res =
        try_s!(req.header(Range::bytes(start_next, end)).header(Connection::close()).send());

      // println!("{}", res.status);
      if res.status.is_server_error() {
        std::thread::sleep(Duration::from_millis(sleep_on_error));
        sleep_on_error = (sleep_on_error as f64 * 1.3) as u64;
        if sleep_on_error > sleep_max {
          sleep_on_error = sleep_max;
        }
        continue;
      }

      loop {
        match res.read(&mut buf[len..]) {
          Ok(0) => {
            // println!("got: {}, expected: {}", len, end - start);
            if (len as u64) < end - start {
              start_next = start + len as u64;
              continue 'outer;
            }

            buf.truncate(len);
            return Ok((start, buf));
          }
          Ok(n) => {
            len += n;
            (self.on_update)(n);
            if len as u64 >= end - start {
              buf.truncate(len);
              return Ok((start, buf));
            }
          }
          // Err(ref e) if e.kind() == ErrorKind::Interrupted => {
          //   break;
          // }
          Err(e) => {
            return Err(e.to_string());
          }
        }
      }
    }
  }
}

fn run() -> Result<(), String> {
  let matches = App::new("drust")
    .version("0.0.1")
    .author("Kevin Darlington <kevin@outroot.com>")
    .about("Downloader")
    .arg(Arg::with_name("output")
      .short("o")
      .long("output")
      .value_name("FILE")
      .help("Sets the output file")
      .takes_value(true))
    .arg(Arg::with_name("threads")
      .long("threads")
      .value_name("COUNT")
      .help("Max amount of threads to use to download in parallel"))
    .arg(Arg::with_name("chunk size")
      .long("chunk-size")
      .value_name("SIZE")
      .help("Specify the max chunk size downloaded at a time. (can use KB, MB, KiB, MiB etc..)"))
    .arg(Arg::with_name("URI")
      .help("Sets the URI to download from")
      .required(true)
      .index(1))
    .get_matches();

  let uri = matches.value_of("URI").unwrap();
  let output = if matches.is_present("output") {
    matches.value_of("output").unwrap().to_owned()
  } else {
    let u = try_s!(Url::parse(uri));
    let p = Path::new(u.path());

    p.file_name().unwrap().to_str().unwrap().to_owned()
  };

  let size = try_s!(check(uri));
  let chunk_size = if matches.is_present("chunk-size") {
    parse_bytes(matches.value_of("chunk-size").unwrap()).unwrap()
  } else {
    5 * 1024 * 1024
  };

  // try_s!(download(uri, output, size, &mut pb));
  let mut pb = ProgressBar::new(size);
  pb.set_units(Units::Bytes);
  pb.format("╢=> ╟");
  pb.add(0);

  let pb = Arc::new(Mutex::new(pb)).clone();
  let dl = Download {
    url: uri.to_owned(),
    output: output,
    size: size,
    chunk_size: chunk_size,
    max_threads: matches.value_of("threads").unwrap_or("10").parse::<usize>().unwrap(),
    on_update: Box::new(move |s| {
      let mut pb = pb.lock().unwrap();
      pb.add(s as u64);
    }),
  };

  dl.run()
}

fn main() {
  TermLogger::init(LogLevelFilter::Info).unwrap();

  match run() {
    Ok(_) => (),
    Err(e) => {
      error!("{}", e);
      exit(-1);
    }
  };
}
