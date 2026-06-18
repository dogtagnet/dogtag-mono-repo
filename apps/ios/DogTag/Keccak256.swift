import Foundation

/// Keccak-256 (Ethereum variant, NOT NIST SHA3-256 — padding byte 0x01, not 0x06).
/// Pure-Swift reference implementation, used for the eth-address derivation.
enum Keccak256 {
    private static let rounds = 24
    private static let rc: [UInt64] = [
        0x0000000000000001, 0x0000000000008082, 0x800000000000808a, 0x8000000080008000,
        0x000000000000808b, 0x0000000080000001, 0x8000000080008081, 0x8000000000008009,
        0x000000000000008a, 0x0000000000000088, 0x0000000080008009, 0x000000008000000a,
        0x000000008000808b, 0x800000000000008b, 0x8000000000008089, 0x8000000000008003,
        0x8000000000008002, 0x8000000000000080, 0x000000000000800a, 0x800000008000000a,
        0x8000000080008081, 0x8000000000008080, 0x0000000080000001, 0x8000000080008008,
    ]
    // Rotation offsets indexed by lane (x + 5y).
    private static let rot: [Int] = [
        0, 1, 62, 28, 27,
        36, 44, 6, 55, 20,
        3, 10, 43, 25, 39,
        41, 45, 15, 21, 8,
        18, 2, 61, 56, 14,
    ]

    static func digest(_ input: Data) -> Data {
        let rate = 136 // 1088 bits for keccak-256
        var state = [UInt64](repeating: 0, count: 25)

        var msg = [UInt8](input)
        // Ethereum keccak padding: 0x01 ... 0x80.
        let padLen = rate - (msg.count % rate)
        var pad = [UInt8](repeating: 0, count: padLen)
        pad[0] = 0x01
        pad[padLen - 1] |= 0x80
        msg.append(contentsOf: pad)

        var offset = 0
        while offset < msg.count {
            for i in 0..<(rate / 8) {
                var lane: UInt64 = 0
                for b in 0..<8 { lane |= UInt64(msg[offset + i*8 + b]) << UInt64(8*b) }
                state[i] ^= lane
            }
            keccakF(&state)
            offset += rate
        }

        var out = Data()
        for i in 0..<4 {
            let lane = state[i]
            for b in 0..<8 { out.append(UInt8((lane >> UInt64(8*b)) & 0xFF)) }
        }
        return out
    }

    private static func keccakF(_ a: inout [UInt64]) {
        for round in 0..<rounds {
            // Theta
            var c = [UInt64](repeating: 0, count: 5)
            for x in 0..<5 { c[x] = a[x] ^ a[x+5] ^ a[x+10] ^ a[x+15] ^ a[x+20] }
            var d = [UInt64](repeating: 0, count: 5)
            for x in 0..<5 { d[x] = c[(x+4)%5] ^ rotl(c[(x+1)%5], 1) }
            for x in 0..<5 { for y in 0..<5 { a[x + 5*y] ^= d[x] } }

            // Rho + Pi: b[y, 2x+3y] = rot(a[x,y], rot[x,y])
            var b = [UInt64](repeating: 0, count: 25)
            for x in 0..<5 {
                for y in 0..<5 {
                    let nx = y
                    let ny = (2*x + 3*y) % 5
                    b[nx + 5*ny] = rotl(a[x + 5*y], rot[x + 5*y])
                }
            }

            // Chi
            for y in 0..<5 {
                for x in 0..<5 {
                    a[x + 5*y] = b[x + 5*y] ^ ((~b[(x+1)%5 + 5*y]) & b[(x+2)%5 + 5*y])
                }
            }

            // Iota
            a[0] ^= rc[round]
        }
    }

    private static func rotl(_ v: UInt64, _ n: Int) -> UInt64 {
        n == 0 ? v : (v << UInt64(n)) | (v >> UInt64(64 - n))
    }
}
