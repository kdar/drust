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

macro_rules! try_s {
  ($expr:expr) => (match $expr {
    std::result::Result::Ok(val) => val,
    std::result::Result::Err(err) => {
      return std::result::Result::Err(err.to_string());
    }
  })
}

fn check<U: IntoUrl>(url: U) -> Result<u64, String> {
  let client = Client::new();
  let res = try_s!(client.get(url)
    .send());

  println!("{:?}", res.headers);

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
    let mut f = File::create(self.output.clone()).unwrap();

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

    cue::pipeline("download",
                  self.max_threads,
                  work.iter(),
                  |item| self.get_part(item.0, item.1),
                  |r| {
      let data = match r {
        Ok(d) => d,
        Err(e) => {
          error!("{}", e);
          return;
        }
      };
      f.seek(SeekFrom::Start(data.0)).unwrap();
      f.write(data.1.as_slice()).unwrap();
    });

    Ok(())
  }

  fn get_part(&self, start: u64, end: u64) -> Result<(u64, Vec<u8>), String> {
    let size = end - start;

    let client = Client::new();
    let req = client.get(&self.url.clone());
    let mut res = try_s!(req.header(Range::bytes(start, end)).header(Connection::close()).send());

    let mut buf: Vec<u8> = vec![0; size as usize];
    let mut len = 0;

    loop {
      match res.read(&mut buf[len..]) {
        Ok(0) => {
          buf.truncate(len);
          return Ok((start, buf));
        }
        Ok(n) => {
          len += n;
          (self.on_update)(n);
          if len as u64 >= size {
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
    chunk_size: 10 * 1024 * 1024,
    max_threads: 12,
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
