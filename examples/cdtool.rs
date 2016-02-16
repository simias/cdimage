extern crate cdimage;

use std::path::Path;

use cdimage::Image;

fn main() {
    let argv: Vec<_> = std::env::args().collect();

    if argv.len() < 2 {
        panic!("Usage: cdtool <cd-image>");
    }

    match cdimage::cue::Cue::new(Path::new(&argv[1])) {
        Ok(c) => {
            println!("Cue creation ok {}", c.image_format());
            println!("{:?}", c);
        }
        Err(e) => println!("Cue error: {}", e),
    }
}
