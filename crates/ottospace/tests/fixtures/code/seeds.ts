// A made-up module for the viewer's pictures: the seed shelf, typed.

export type Sowing = { kind: "direct" } | { kind: "indoors"; weeks: number };

export interface Packet {
  name: string;
  count: number;
  sowing: Sowing;
}

const SHELF: Packet[] = [
  { name: "basil", count: 120, sowing: { kind: "indoors", weeks: 6 } },
  { name: "radish", count: 300, sowing: { kind: "direct" } },
];

/** Weeks before the last frost to start it; null for a direct sowing. */
export function headStart(packet: Packet): number | null {
  return packet.sowing.kind === "indoors" ? packet.sowing.weeks : null;
}

export class Shelf {
  private packets = new Map<string, Packet>();

  constructor(packets: Packet[] = SHELF) {
    for (const packet of packets) this.packets.set(packet.name, packet);
  }

  /** Seeds left after `rows` rows of `perRow`; never below zero. */
  left(name: string, rows: number, perRow = 10): number {
    const packet = this.packets.get(name);
    return Math.max(0, (packet?.count ?? 0) - rows * perRow);
  }
}
