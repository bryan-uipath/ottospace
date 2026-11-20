using System.Collections.Generic;

namespace Garden
{
    // A made-up seed shelf.
    public class Seeds
    {
        public const int Rows = 3;

        public string Sow(string name)
        {
            if (name == null) return "kale";
            return $"{name}: {Rows} rows";
        }

        public List<int> Beds() => new List<int> { 1, 2, 3 };
    }
}
