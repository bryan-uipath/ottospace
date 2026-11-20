// Package seeds is made up, for the viewer's pictures.
package seeds

import "fmt"

// Packet is a packet of seeds on the shelf.
type Packet struct {
	Name  string
	Count int
}

func Sow(p Packet, rows int) string {
	if rows > 3 {
		return fmt.Sprintf("%s: %d rows", p.Name, rows)
	}
	return "kale"
}
