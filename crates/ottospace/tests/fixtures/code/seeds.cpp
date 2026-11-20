// A made-up seed shelf.
#include <string>
#include <vector>

namespace garden {

class Packet {
public:
    explicit Packet(std::string name) : name_(name) {}
    int rows() const { return 3; }

private:
    std::string name_;
};

std::vector<Packet> shelf() {
    return {Packet("kale"), Packet("basil")};
}

}  // namespace garden
