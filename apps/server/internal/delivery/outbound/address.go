package outbound

import "net/netip"

// IsPublicAddress preserves the original worker's explicit IPv4 and IPv6 destination policy.
func IsPublicAddress(address netip.Addr) bool {
	if !address.IsValid() {
		return false
	}
	if address.Is4In6() {
		address = address.Unmap()
	}
	if address.Is6() {
		return isPublicIpv6(address)
	}
	return isPublicIpv4(address)

}

func isPublicIpv6(address netip.Addr) bool {
	bytes := address.As16()
	if isIpv4Compatible(bytes) {
		return IsPublicAddress(netip.AddrFrom4([4]byte{bytes[12], bytes[13], bytes[14], bytes[15]}))
	}
	first := uint16(bytes[0])<<8 | uint16(bytes[1])
	second := uint16(bytes[2])<<8 | uint16(bytes[3])
	third := uint16(bytes[4])<<8 | uint16(bytes[5])
	return first&0xe000 == 0x2000 && !isIpv6ProtocolAssignment(first, second) && !isIpv6Documentation(first, second) && first != 0x2002 && !(first == 0x2620 && second == 0x004f && third == 0x8000)
}

func isIpv4Compatible(bytes [16]byte) bool {
	for _, value := range bytes[:12] {
		if value != 0 {
			return false
		}
	}
	return true
}

func isIpv6ProtocolAssignment(first, second uint16) bool {
	return first == 0x2001 && second <= 0x01ff
}

func isIpv6Documentation(first, second uint16) bool {
	return first == 0x2001 && second == 0x0db8 || first == 0x3fff && second&0xf000 == 0
}

func isPublicIpv4(address netip.Addr) bool {
	bytes := address.As4()
	return !(address.IsUnspecified() || address.IsLoopback() || address.IsPrivate() || address.IsLinkLocalUnicast() || address.IsMulticast() || isIpv4SharedOrProtocol(bytes) || isIpv4BenchmarkOrReserved(bytes) || isIpv4Documentation(bytes))
}

func isIpv4SharedOrProtocol(bytes [4]byte) bool {
	return bytes[0] == 0 || bytes[0] == 100 && bytes[1] >= 64 && bytes[1] <= 127 || bytes[0] == 192 && bytes[1] == 0 && bytes[2] == 0
}

func isIpv4BenchmarkOrReserved(bytes [4]byte) bool {
	return bytes == [4]byte{255, 255, 255, 255} || bytes[0] == 198 && (bytes[1] == 18 || bytes[1] == 19) || bytes[0] >= 240
}

func isIpv4Documentation(bytes [4]byte) bool {
	return bytes[0] == 192 && bytes[1] == 0 && bytes[2] == 2 || bytes[0] == 198 && bytes[1] == 51 && bytes[2] == 100 || bytes[0] == 203 && bytes[1] == 0 && bytes[2] == 113
}
