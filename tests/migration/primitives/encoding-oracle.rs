//! Export exhaustive GB18030 code-unit results from the frozen Rust dependency.
use std::fs::File;
use std::io::{BufWriter, Write};

fn main() -> std::io::Result<()> {
    let destination = std::env::args().nth(1).expect("output path required");
    let mut output = BufWriter::new(File::create(destination)?);
    output.write_all(b"LRGB1801")?;
    for first in 0x81u8..=0xfe {
        for second in (0x40u8..=0xfe).filter(|value| *value != 0x7f) {
            write_unit(&mut output, &[first, second])?;
        }
    }
    for first in 0x81u8..=0xfe {
        for second in 0x30u8..=0x39 {
            for third in 0x81u8..=0xfe {
                for fourth in 0x30u8..=0x39 {
                    write_unit(&mut output, &[first, second, third, fourth])?;
                }
            }
        }
    }
    output.flush()
}

fn write_unit(output: &mut impl Write, unit: &[u8]) -> std::io::Result<()> {
    let scalar = encoding_rs::GB18030
        .decode_without_bom_handling_and_without_replacement(unit)
        .map(|value| {
            let mut characters = value.chars();
            let first = characters.next().expect("decoded scalar");
            assert!(characters.next().is_none());
            first as i32
        })
        .unwrap_or(-1);
    output.write_all(&scalar.to_le_bytes())
}
