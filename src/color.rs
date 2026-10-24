pub fn distance(a: [u8; 3], b: [u8; 3]) -> u32 {
    a.into_iter()
        .zip(b)
        .map(|(a, b)| {
            let delta = i32::from(a) - i32::from(b);
            (delta * delta) as u32
        })
        .sum()
}

pub fn nearest(color: [u8; 3], palette: &[[u8; 3]]) -> Option<[u8; 3]> {
    palette
        .iter()
        .min_by_key(|entry| distance(color, **entry))
        .copied()
}

pub fn encode_rgba(rgba: [u8; 4]) -> i32 {
    i32::from_be_bytes([rgba[3], rgba[0], rgba[1], rgba[2]])
}

pub fn decode_rgba(value: i32) -> [u8; 4] {
    let [a, r, g, b] = value.to_be_bytes();
    [r, g, b, a]
}
