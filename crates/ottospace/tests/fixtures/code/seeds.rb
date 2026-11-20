# A made-up seed shelf.
class Packet
  ROWS = 3

  def initialize(name)
    @name = name
  end

  def sow(rows = ROWS)
    return "kale" if rows.zero?
    "#{@name}: #{rows} rows"
  end
end
