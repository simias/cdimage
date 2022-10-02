extern crate cdimage;

use std::path::Path;
use std::str::FromStr;

use cdimage::msf::Msf;
use cdimage::Image;

fn main() {
    let argv: Vec<_> = std::env::args().collect();

    if argv.len() < 2 {
        panic!("Usage: cdtool <cd-image> [msf]");
    }

    let file = Path::new(&argv[1]);

    let img = if file.extension().and_then(|ext| ext.to_str()) == Some("cue") {
        cdimage::cue::Cue::new(file)
    } else {
        cdimage::cue::Cue::new_from_zip(file)
    };

    let mut img = img.unwrap_or_else(|e| panic!("Cue error: {}", e));

    println!("{:?}", img.toc());

    if argv.len() >= 3 {
        let msf = &argv[2];
        let msf = Msf::from_str(msf).unwrap();

        let sector = img.read_sector(msf.to_disc_position()).unwrap();

        if let Ok(xa_subheader) = sector.mode2_xa_subheader() {
            println!("XA Mode 2 form: {:?}", xa_subheader.submode().form());
        }

        let bytes = sector.data_2352();

        hexdump(bytes);
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
