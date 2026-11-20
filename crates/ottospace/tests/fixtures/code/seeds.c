/* A made-up seed shelf. */
#include <stdio.h>

typedef struct {
    const char *name;
    int count;
} Packet;

// Prints a packet and its rows.
int sow(Packet packet, int rows) {
    for (int i = 0; i < rows; i++) {
        printf("%s %d\n", packet.name, 40);
    }
    return 0;
}
