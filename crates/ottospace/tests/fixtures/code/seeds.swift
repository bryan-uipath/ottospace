// A made-up seed shelf.
import Foundation

struct Packet {
    let name: String
    var count: Int
}

func sow(_ packet: Packet, rows: Int = 3) -> String {
    guard rows > 0 else { return "kale" }
    return "\(packet.name): \(rows) rows"
}
