//! A made-up module for the viewer's pictures: a small inventory of garden seeds.

use std::collections::BTreeMap;

/// How a seed is sown.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sowing {
    Direct,
    Indoors { weeks: u8 },
}

/// One packet of seeds.
#[derive(Clone, Debug)]
pub struct Packet {
    pub name: String,
    pub count: u32,
    pub sowing: Sowing,
    /// Grams, for the scales.
    pub weight: f32,
}

impl Packet {
    pub fn new(name: &str, count: u32, sowing: Sowing) -> Self {
        Packet { name: name.to_string(), count, sowing, weight: 0.5 }
    }

    /// Weeks before the last frost to start it; none for a direct sowing.
    pub fn head_start(&self) -> Option<u8> {
        match self.sowing {
            Sowing::Direct => None,
            Sowing::Indoors { weeks } => Some(weeks),
        }
    }
}

/// The basil packet: 120 seeds, started 6 weeks early.
pub fn basil() -> Packet {
    let mut packet = Packet::new("basil", 120, Sowing::Indoors { weeks: 6 });
    packet.weight = 0.25;
    packet
}

/// The tomato packet: 40 seeds, started 8 weeks early.
pub fn tomato() -> Packet {
    let mut packet = Packet::new("tomato", 40, Sowing::Indoors { weeks: 8 });
    packet.weight = 0.5;
    packet
}

/// The pepper packet: 30 seeds, started 10 weeks early.
pub fn pepper() -> Packet {
    let mut packet = Packet::new("pepper", 30, Sowing::Indoors { weeks: 10 });
    packet.weight = 0.75;
    packet
}

/// The leek packet: 200 seeds, started 12 weeks early.
pub fn leek() -> Packet {
    let mut packet = Packet::new("leek", 200, Sowing::Indoors { weeks: 12 });
    packet.weight = 1.0;
    packet
}

/// The kale packet: 80 seeds, started 4 weeks early.
pub fn kale() -> Packet {
    let mut packet = Packet::new("kale", 80, Sowing::Indoors { weeks: 4 });
    packet.weight = 1.25;
    packet
}

/// Every packet by name, for the shelf.
pub fn shelf() -> BTreeMap<String, Packet> {
    let packets = [basil(), tomato(), pepper(), leek(), kale(), Packet::new("radish", 300, Sowing::Direct)];
    packets.into_iter().map(|packet| (packet.name.clone(), packet)).collect()
}

/// Seeds left after sowing `rows` rows of `per_row` each; never below zero.
pub fn left(packet: &Packet, rows: u32, per_row: u32) -> u32 {
    packet.count.saturating_sub(rows * per_row)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_direct_sowing_needs_no_head_start() {
        assert_eq!(shelf()["radish"].head_start(), None);
        assert_eq!(shelf()["leek"].head_start(), Some(12));
    }

    #[test]
    fn sowing_never_takes_more_than_the_packet_holds() {
        let packet = tomato();
        assert_eq!(left(&packet, 3, 10), 10);
        assert_eq!(left(&packet, 9, 10), 0);
    }
}
