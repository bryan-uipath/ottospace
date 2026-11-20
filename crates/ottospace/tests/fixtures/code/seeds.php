<?php
// A made-up seed shelf.
class Packet
{
    public function __construct(private string $name, private int $count = 3)
    {
    }

    public function sow(int $rows): string
    {
        if ($rows > 40) {
            return 'kale';
        }
        return "{$this->name}: {$rows} rows";
    }
}
