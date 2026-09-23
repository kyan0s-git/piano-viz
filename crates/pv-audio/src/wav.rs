//! WAV output: 32-bit float, the lossless intermediate handed to the muxer.

use std::io::Write;

pub fn write_f32<W: Write>(
    mut w: W,
    interleaved: &[f32],
    channels: u16,
    sample_rate: u32,
) -> std::io::Result<()> {
    let data_len = (interleaved.len() * 4) as u32;
    let block = channels * 4;
    w.write_all(b"RIFF")?;
    w.write_all(&(36 + data_len).to_le_bytes())?;
    w.write_all(b"WAVEfmt ")?;
    w.write_all(&16u32.to_le_bytes())?;
    w.write_all(&3u16.to_le_bytes())?; // IEEE float
    w.write_all(&channels.to_le_bytes())?;
    w.write_all(&sample_rate.to_le_bytes())?;
    w.write_all(&(sample_rate * block as u32).to_le_bytes())?;
    w.write_all(&block.to_le_bytes())?;
    w.write_all(&32u16.to_le_bytes())?;
    w.write_all(b"data")?;
    w.write_all(&data_len.to_le_bytes())?;
    let mut buf = Vec::with_capacity(interleaved.len().min(1 << 16) * 4);
    for chunk in interleaved.chunks(1 << 16) {
        buf.clear();
        for v in chunk {
            buf.extend_from_slice(&v.to_le_bytes());
        }
        w.write_all(&buf)?;
    }
    w.flush()
}
