package garden;

import java.util.List;

/** A made-up seed shelf. */
public class Seeds {
    private static final int ROWS = 3;

    // Each packet's name.
    public List<String> names() {
        return List.of("kale", "basil");
    }

    public int sow(int rows) {
        return rows > ROWS ? ROWS : rows;
    }
}
