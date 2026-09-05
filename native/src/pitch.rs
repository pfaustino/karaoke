pub const NOTE_NAMES: [&str; 12] = ["C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B"];

pub const SCALE_STEPS: [&[i32]; 3] = [
    &[0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11],
    &[0, 2, 4, 5, 7, 9, 11],
    &[0, 2, 3, 5, 7, 8, 10],
];

const A4_HZ: f32 = 440.0;
const A4_MIDI: f32 = 69.0;
const SEARCH_RADIUS: i32 = 12;
const MIN_FREQ: f32 = 70.0;
const MAX_FREQ: f32 = 1200.0;
const MIN_RATIO: f32 = 0.5;
const MAX_RATIO: f32 = 2.0;

pub fn freq_to_midi(freq: f32) -> f32 {
    A4_MIDI + 12.0 * (freq / A4_HZ).log2()
}

pub fn midi_to_freq(midi: f32) -> f32 {
    A4_HZ * 2f32.powf((midi - A4_MIDI) / 12.0)
}

pub fn midi_to_note_name(midi: f32) -> String {
    let rounded = midi.round() as i32;
    let pc = ((rounded % 12) + 12) % 12;
    let octave = rounded.div_euclid(12) - 1;
    format!("{}{octave}", NOTE_NAMES[pc as usize])
}

pub fn snap_midi(midi: f32, key: i32, scale: usize) -> i32 {
    let tonic = ((key % 12) + 12) % 12;
    let steps = SCALE_STEPS[scale.min(2)];
    let lo = midi.floor() as i32 - SEARCH_RADIUS;
    let hi = midi.floor() as i32 + SEARCH_RADIUS;
    let mut best_midi = midi.round() as i32;
    let mut best_dist = f32::INFINITY;

    for candidate in lo..=hi {
        let pitch_class = ((candidate % 12) + 12) % 12;
        let degree = (pitch_class - tonic + 12) % 12;
        if !steps.contains(&degree) {
            continue;
        }
        let dist = (candidate as f32 - midi).abs();
        if dist < best_dist {
            best_dist = dist;
            best_midi = candidate;
        }
    }
    best_midi
}

pub fn correction_ratio(freq: f32, key: i32, scale: usize) -> f32 {
    if !freq.is_finite() || freq < MIN_FREQ || freq > MAX_FREQ {
        return 1.0;
    }
    let midi = freq_to_midi(freq);
    let target = snap_midi(midi, key, scale) as f32;
    let ratio = midi_to_freq(target) / freq;
    ratio.clamp(MIN_RATIO, MAX_RATIO)
}

pub fn cents_off(freq: f32, target_midi: f32) -> f32 {
    if !freq.is_finite() || freq <= 0.0 {
        return 0.0;
    }
    100.0 * (freq_to_midi(freq) - target_midi)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_a4() {
        assert!((freq_to_midi(440.0) - 69.0).abs() < 1e-5);
        assert!((midi_to_freq(69.0) - 440.0).abs() < 1e-3);
        assert_eq!(midi_to_note_name(69.0), "A4");
    }

    #[test]
    fn snaps_sharp_c_to_c_major() {
        assert_eq!(snap_midi(60.4, 0, 1), 60);
    }

    #[test]
    fn snaps_toward_d_in_c_major() {
        assert_eq!(snap_midi(61.2, 0, 1), 62);
    }

    #[test]
    fn keeps_c_sharp_in_chromatic() {
        assert_eq!(snap_midi(61.1, 0, 0), 61);
    }

    #[test]
    fn corrects_466_toward_a_in_major() {
        let ratio = correction_ratio(466.16, 0, 1);
        assert!((ratio - 440.0 / 466.16).abs() < 0.01);
        let chromatic = correction_ratio(466.16, 0, 0);
        assert!((chromatic - 1.0).abs() < 0.05);
    }

    #[test]
    fn unity_for_silence() {
        assert!((correction_ratio(0.0, 0, 1) - 1.0).abs() < f32::EPSILON);
        assert!((correction_ratio(40.0, 0, 1) - 1.0).abs() < f32::EPSILON);
    }

    #[test]
    fn reports_cents() {
        assert!(cents_off(440.0, 69.0).abs() < 0.01);
        assert!((cents_off(midi_to_freq(70.0), 69.0) - 100.0).abs() < 0.05);
    }
}
