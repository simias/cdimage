extern crate cdimage;

use std::path::Path;
use std::str::FromStr;

use cdimage::Image;
use cdimage::msf::Msf;
use cdimage::sector::Sector;

fn main() {
    let argv: Vec<_> = std::env::args().collect();

    if argv.len() < 3 {
        panic!("Usage: cdtool <cd-image> <msf>");
    }

    let file = &argv[1];
    let msf = &argv[2];

    let msf =
        match Msf::from_str(msf) {
            Ok(m) => m,
            Err(()) => panic!("Invalid MSF"),
        };

    match cdimage::cue::Cue::new(Path::new(file)) {
        Ok(mut c) => {
            println!("{:?}", c);

            let mut sector = Sector::empty();

            c.read_sector(&mut sector, msf).unwrap();

            let bytes = sector.data_2352().unwrap();

            hexdump(bytes);
        }
        Err(e) => println!("Cue error: {}", e),
    }
}

fn hexdump(bytes: &[u8]) {
    fn is_print(b: u8) -> bool {
        b >= b' ' && b <= b'~'
    }

    let mut pos = 0;

    while pos + 16 <= bytes.len() {
        let bytes = &bytes[pos..pos+16];

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
