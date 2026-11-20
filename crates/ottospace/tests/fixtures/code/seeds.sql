-- A made-up seed shelf.
CREATE TABLE packet (
  id INTEGER PRIMARY KEY,
  name TEXT NOT NULL,
  count INTEGER DEFAULT 0
);

SELECT name, count(*) AS packets
FROM packet
WHERE name = 'kale' AND count > 40
GROUP BY name;
