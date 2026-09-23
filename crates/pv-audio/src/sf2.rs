//! A minimal SoundFont 2 writer: one preset over one instrument, mono
//! 16-bit samples. Enough to package the generated piano so the same
//! synthesizer plays it and any SoundFont a user loads.

/// SF2 generator operators used here (SF2.01 §8.1.2).
pub mod generator {
    pub const CHORUS_SEND: u16 = 15;
    pub const REVERB_SEND: u16 = 16;
    pub const PAN: u16 = 17;
    pub const ATTACK_VOL_ENV: u16 = 34;
    pub const DECAY_VOL_ENV: u16 = 36;
    pub const SUSTAIN_VOL_ENV: u16 = 37;
    pub const RELEASE_VOL_ENV: u16 = 38;
    pub const INSTRUMENT: u16 = 41;
    pub const KEY_RANGE: u16 = 43;
    pub const VEL_RANGE: u16 = 44;
    pub const INITIAL_ATTENUATION: u16 = 48;
    pub const SAMPLE_ID: u16 = 53;
    pub const SAMPLE_MODES: u16 = 54;
}

pub struct Sample {
    pub name: String,
    pub data: Vec<i16>,
    pub rate: u32,
    pub root: u8,
}

pub struct Zone {
    pub keys: (u8, u8),
    pub velocities: (u8, u8),
    pub sample: usize,
    /// Other generators, applied between the ranges and the sample ID.
    pub generators: Vec<(u16, i16)>,
}

/// Seconds as SF2 timecents.
pub fn timecents(seconds: f32) -> i16 {
    (1200.0 * seconds.max(0.001).log2()).round().clamp(-12000.0, 8000.0) as i16
}

/// The spec requires at least 46 zero samples after each sample.
const SAMPLE_GAP: usize = 46;

pub fn build(
    name: &str,
    samples: &[Sample],
    zones: &[Zone],
    preset_generators: &[(u16, i16)],
) -> Vec<u8> {
    // ── INFO ──
    let mut info = Vec::new();
    info.extend(chunk(b"ifil", &[2, 0, 1, 0]));
    info.extend(chunk(b"isng", &zstr("EMU8000", 8)));
    info.extend(chunk(b"INAM", &zstr(name, name.len() + 2)));

    // ── sdta ──
    let mut smpl = Vec::new();
    let mut spans = Vec::new();
    for s in samples {
        let start = smpl.len() / 2;
        for v in &s.data {
            smpl.extend_from_slice(&v.to_le_bytes());
        }
        let end = smpl.len() / 2;
        smpl.extend(std::iter::repeat_n(0u8, SAMPLE_GAP * 2));
        spans.push((start as u32, end as u32));
    }
    let sdta = chunk(b"smpl", &smpl);

    // ── pdta ──
    let mut phdr = Vec::new();
    phdr.extend(preset_header(name, 0, 0, 0));
    phdr.extend(preset_header("EOP", 255, 255, 1));

    let preset_gens: Vec<(u16, i16)> =
        preset_generators.iter().copied().chain([(generator::INSTRUMENT, 0)]).collect();
    let mut pbag = bag(0);
    pbag.extend(bag(preset_gens.len() as u16));
    let mut pgen: Vec<u8> = preset_gens.iter().flat_map(|&(o, a)| gen_bytes(o, a as u16)).collect();
    pgen.extend([0; 4]);

    let mut inst = Vec::new();
    inst.extend(zstr(name, 20));
    inst.extend(0u16.to_le_bytes());
    inst.extend(zstr("EOI", 20));
    inst.extend((zones.len() as u16).to_le_bytes());

    let mut ibag = Vec::new();
    let mut igen = Vec::new();
    let mut gen_count = 0u16;
    for z in zones {
        ibag.extend(bag(gen_count));
        // Order matters: key range first, velocity range second, sample last.
        let mut gens = vec![
            (generator::KEY_RANGE, u16::from_le_bytes([z.keys.0, z.keys.1])),
            (generator::VEL_RANGE, u16::from_le_bytes([z.velocities.0, z.velocities.1])),
        ];
        gens.extend(z.generators.iter().map(|&(o, a)| (o, a as u16)));
        gens.push((generator::SAMPLE_ID, z.sample as u16));
        for (o, a) in gens {
            igen.extend(gen_bytes(o, a));
            gen_count += 1;
        }
    }
    ibag.extend(bag(gen_count));
    igen.extend([0; 4]);

    let mut shdr = Vec::new();
    for (s, &(start, end)) in samples.iter().zip(&spans) {
        shdr.extend(zstr(&s.name, 20));
        for v in [start, end, start, end, s.rate] {
            shdr.extend(v.to_le_bytes());
        }
        shdr.extend([s.root, 0]);
        shdr.extend(0u16.to_le_bytes()); // sample link
        shdr.extend(1u16.to_le_bytes()); // mono
    }
    shdr.extend([0u8; 46].iter().enumerate().map(|(i, _)| if i < 3 { b"EOS"[i] } else { 0 }));

    let mut pdta = Vec::new();
    for (id, data) in [
        (b"phdr", phdr),
        (b"pbag", pbag),
        (b"pmod", vec![0; 10]),
        (b"pgen", pgen),
        (b"inst", inst),
        (b"ibag", ibag),
        (b"imod", vec![0; 10]),
        (b"igen", igen),
        (b"shdr", shdr),
    ] {
        pdta.extend(chunk(id, &data));
    }

    let mut body = b"sfbk".to_vec();
    body.extend(list(b"INFO", &info));
    body.extend(list(b"sdta", &sdta));
    body.extend(list(b"pdta", &pdta));
    chunk(b"RIFF", &body)
}

fn chunk(id: &[u8; 4], data: &[u8]) -> Vec<u8> {
    let mut v = id.to_vec();
    v.extend((data.len() as u32).to_le_bytes());
    v.extend_from_slice(data);
    if data.len() % 2 == 1 {
        v.push(0);
    }
    v
}

fn list(kind: &[u8; 4], data: &[u8]) -> Vec<u8> {
    let mut body = kind.to_vec();
    body.extend_from_slice(data);
    chunk(b"LIST", &body)
}

/// Zero-terminated, zero-padded fixed-length string.
fn zstr(s: &str, len: usize) -> Vec<u8> {
    let mut v: Vec<u8> = s.bytes().filter(u8::is_ascii).take(len.saturating_sub(1)).collect();
    v.resize(len, 0);
    if v.len() % 2 == 1 {
        v.push(0);
    }
    v
}

fn preset_header(name: &str, preset: u16, bank: u16, bag: u16) -> Vec<u8> {
    let mut v = zstr(name, 20);
    v.extend(preset.to_le_bytes());
    v.extend(bank.to_le_bytes());
    v.extend(bag.to_le_bytes());
    v.extend([0u8; 12]); // library, genre, morphology
    v
}

fn bag(gen_index: u16) -> Vec<u8> {
    let mut v = gen_index.to_le_bytes().to_vec();
    v.extend(0u16.to_le_bytes());
    v
}

fn gen_bytes(oper: u16, amount: u16) -> [u8; 4] {
    let (o, a) = (oper.to_le_bytes(), amount.to_le_bytes());
    [o[0], o[1], a[0], a[1]]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn minimal_font_loads_in_rustysynth() {
        let data: Vec<i16> =
            (0..4000).map(|i| ((i as f32 * 0.05).sin() * 12000.0) as i16).collect();
        let samples = [Sample { name: "sine".into(), data, rate: 32000, root: 69 }];
        let zones = [Zone { keys: (0, 127), velocities: (0, 127), sample: 0, generators: vec![] }];
        let bytes = build("Test", &samples, &zones, &[]);
        let sf = rustysynth::SoundFont::new(&mut std::io::Cursor::new(&bytes)).expect("parses");
        assert_eq!(sf.get_presets().len(), 1);
        assert_eq!(sf.get_sample_headers().len(), 1);
    }

    #[test]
    fn timecents_round_trip() {
        assert_eq!(timecents(1.0), 0);
        assert_eq!(timecents(2.0), 1200);
        assert_eq!(timecents(0.5), -1200);
    }
}
