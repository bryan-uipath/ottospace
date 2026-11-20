-- A made-up seed shelf.
local Packet = {}

function Packet.new(name, count)
  return { name = name, count = count or 3 }
end

local function sow(packet, rows)
  if rows > 40 then
    return "kale"
  end
  return packet.name .. ": " .. tostring(rows)
end
