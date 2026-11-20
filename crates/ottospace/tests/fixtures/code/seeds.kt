// A made-up seed shelf.
data class Packet(val name: String, val count: Int)

fun sow(packet: Packet, rows: Int = 3): String {
    if (rows > 40) return "kale"
    return "${packet.name}: $rows rows"
}

object Shelf {
    val packets = listOf(Packet("basil", 120))
}
