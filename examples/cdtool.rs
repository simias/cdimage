extern crate cdimage;
extern crate clap;

use std::path::Path;
use std::str::FromStr;

use cdimage::msf::Msf;
use cdimage::Image;
use clap::{Parser, ValueEnum};
use std::io::Write;

#[derive(Parser)]
#[command(version = "1.0", author = "Your Name", about = "CD image tool")]
struct Cli {
    /// Path to the CD image file
    image: String,

    /// MSF sector to read (optional)
    msf: Option<String>,

    /// Output raw sector data
    #[arg(long)]
    raw: bool,

    /// What to dump (optional)
    #[arg(long, value_enum, default_value_t = DumpType::FullSector)]
    dump: DumpType,
}

#[derive(ValueEnum, Default, PartialEq, Eq, Copy, Clone)]
enum DumpType {
    /// Dump the entire sector data
    #[default]
    FullSector,
    /// Strip the headers and only dump the sector's payload
    Payload,
    /// Dump Q subchannel data only
    SubQ,
}

fn main() {
    let args = Cli::parse();
    let file = Path::new(&args.image);

    let img = if file.extension().and_then(|ext| ext.to_str()) == Some("cue") {
        cdimage::cue::Cue::new(file)
    } else {
        cdimage::cue::Cue::new_from_zip(file)
    };

    let mut img = img.unwrap_or_else(|e| panic!("Cue error: {}", e));

    if !args.raw {
        println!("{:?}", img.toc());
    }

    if let Some(msf_str) = args.msf {
        let msf = Msf::from_str(&msf_str).unwrap();
        let sector = img.read_sector(msf.to_disc_position()).unwrap();

        if !args.raw {
            if let Ok(xa_subheader) = sector.mode2_xa_subheader() {
                println!("XA Mode 2 form: {:?}", xa_subheader.submode().form());
            }
        }
        let bytes = match args.dump {
            DumpType::FullSector => sector.data_2352(),
            DumpType::Payload => sector.mode2_xa_payload().unwrap(),
            DumpType::SubQ => &sector.q().to_raw(),
        };

        if args.raw {
            std::io::stdout().write_all(bytes).unwrap();
        } else {
            hexdump(bytes);
        }
    }
}

fn hexdump(bytes: &[u8]) {
    fn is_print(b: u8) -> bool {
        b >= b' ' && b <= b'~'
    }

    let mut pos = 0;

    while pos + 16 <= bytes.len() {
        let bytes = &bytes[pos..pos + 16];

        print!("{:08x}  ", pos);

        for &b in &bytes[0..8] {
            print!("{:02x} ", b)
        }

        print!(" ");

        for &b in &bytes[8..16] {
            print!("{:02x} ", b)
        }

        print!(" |");

        for &b in &bytes[0..16] {
            if is_print(b) {
                print!("{}", b as char);
            } else {
                print!(".");
            }
        }

        println!("|");

        pos += 16;
    }

    let rem = bytes.len() & !15;

    if rem != bytes.len() {
        print!("{:08x} ", rem);

        for p in rem..bytes.len() {
            let b = bytes[p];

            if p % 8 == 0 {
                print!(" ");
            }

            print!("{:02x} ", b)
        }

        let pad = 16 - bytes.len() % 16;

        if pad >= 8 {
            print!(" ");
        }

        for _ in 0..pad {
            print!("   ");
        }

        print!(" |");

        for &b in &bytes[rem..] {
            if is_print(b) {
                print!("{}", b as char);
            } else {
                print!(".");
            }
        }

        println!("|");
    }
}
