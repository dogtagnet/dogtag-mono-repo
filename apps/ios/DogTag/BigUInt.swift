import Foundation

/// A minimal arbitrary-precision unsigned integer over 32-bit limbs (little-endian), sufficient for
/// secp256k1 modular arithmetic. Not constant-time — used for one-shot key derivation only.
struct BigUInt {
    // limbs[0] is least significant.
    private(set) var limbs: [UInt32]

    init(_ v: UInt32) { limbs = v == 0 ? [] : [v]; normalize() }
    init(limbs: [UInt32]) { self.limbs = limbs; normalize() }

    init(hex: String) {
        var h = hex
        if h.hasPrefix("0x") { h = String(h.dropFirst(2)) }
        if h.count % 2 != 0 { h = "0" + h }
        var bytes = [UInt8]()
        var idx = h.startIndex
        while idx < h.endIndex {
            let next = h.index(idx, offsetBy: 2)
            bytes.append(UInt8(h[idx..<next], radix: 16)!)
            idx = next
        }
        self.init(data: Data(bytes))
    }

    /// Big-endian bytes → BigUInt.
    init(data: Data) {
        var l = [UInt32]()
        let bytes = [UInt8](data)
        var i = bytes.count
        while i > 0 {
            let lo = i >= 1 ? UInt32(bytes[i-1]) : 0
            let b1 = i >= 2 ? UInt32(bytes[i-2]) << 8 : 0
            let b2 = i >= 3 ? UInt32(bytes[i-3]) << 16 : 0
            let b3 = i >= 4 ? UInt32(bytes[i-4]) << 24 : 0
            l.append(lo | b1 | b2 | b3)
            i -= 4
        }
        self.init(limbs: l)
    }

    private mutating func normalize() {
        while let last = limbs.last, last == 0 { limbs.removeLast() }
    }

    var isZero: Bool { limbs.isEmpty }

    var bitLength: Int {
        guard let top = limbs.last else { return 0 }
        return (limbs.count - 1) * 32 + (32 - top.leadingZeroBitCount)
    }

    func bit(_ i: Int) -> Bool {
        let limb = i / 32, off = i % 32
        if limb >= limbs.count { return false }
        return (limbs[limb] >> UInt32(off)) & 1 == 1
    }

    func cmp(_ o: BigUInt) -> Int {
        if limbs.count != o.limbs.count { return limbs.count < o.limbs.count ? -1 : 1 }
        var i = limbs.count - 1
        while i >= 0 {
            if limbs[i] != o.limbs[i] { return limbs[i] < o.limbs[i] ? -1 : 1 }
            i -= 1
        }
        return 0
    }
    func eq(_ o: BigUInt) -> Bool { cmp(o) == 0 }

    func add(_ o: BigUInt) -> BigUInt {
        var res = [UInt32]()
        var carry: UInt64 = 0
        let m = max(limbs.count, o.limbs.count)
        for i in 0..<m {
            let a = i < limbs.count ? UInt64(limbs[i]) : 0
            let b = i < o.limbs.count ? UInt64(o.limbs[i]) : 0
            let s = a + b + carry
            res.append(UInt32(s & 0xFFFFFFFF))
            carry = s >> 32
        }
        if carry != 0 { res.append(UInt32(carry)) }
        return BigUInt(limbs: res)
    }

    /// self - o, assuming self >= o.
    func sub(_ o: BigUInt) -> BigUInt {
        var res = [UInt32]()
        var borrow: Int64 = 0
        for i in 0..<limbs.count {
            let a = Int64(limbs[i])
            let b = i < o.limbs.count ? Int64(o.limbs[i]) : 0
            var d = a - b - borrow
            if d < 0 { d += 0x100000000; borrow = 1 } else { borrow = 0 }
            res.append(UInt32(d))
        }
        return BigUInt(limbs: res)
    }

    func mul(_ o: BigUInt) -> BigUInt {
        if isZero || o.isZero { return BigUInt(0) }
        var res = [UInt64](repeating: 0, count: limbs.count + o.limbs.count)
        for i in 0..<limbs.count {
            var carry: UInt64 = 0
            let a = UInt64(limbs[i])
            for j in 0..<o.limbs.count {
                let cur = res[i + j] + a * UInt64(o.limbs[j]) + carry
                res[i + j] = cur & 0xFFFFFFFF
                carry = cur >> 32
            }
            res[i + o.limbs.count] += carry
        }
        return BigUInt(limbs: res.map { UInt32($0 & 0xFFFFFFFF) })
    }

    /// self mod m via long division (schoolbook). m must be nonzero.
    func mod(_ m: BigUInt) -> BigUInt {
        if cmp(m) < 0 { return self }
        if m.isZero { return BigUInt(0) }
        var rem = BigUInt(0)
        var i = bitLength - 1
        while i >= 0 {
            rem = rem.shiftLeft1()
            if bit(i) { rem = rem.add(BigUInt(1)) }
            if rem.cmp(m) >= 0 { rem = rem.sub(m) }
            i -= 1
        }
        return rem
    }

    private func shiftLeft1() -> BigUInt {
        var res = [UInt32]()
        var carry: UInt32 = 0
        for l in limbs {
            res.append((l << 1) | carry)
            carry = l >> 31
        }
        if carry != 0 { res.append(carry) }
        return BigUInt(limbs: res)
    }

    /// Modular inverse via extended binary GCD (m must be odd prime here → invertible).
    func modInverse(_ m: BigUInt) -> BigUInt {
        // Fermat: a^(m-2) mod m, since m is prime (secp256k1 field p).
        let exp = m.sub(BigUInt(2))
        return modPow(exp, m)
    }

    func modPow(_ exp: BigUInt, _ m: BigUInt) -> BigUInt {
        var result = BigUInt(1)
        var base = mod(m)
        let bits = exp.bitLength
        for i in 0..<bits {
            if exp.bit(i) { result = result.mul(base).mod(m) }
            base = base.mul(base).mod(m)
        }
        return result
    }

    /// Big-endian fixed-width bytes.
    func toData(_ width: Int) -> Data {
        var bytes = [UInt8](repeating: 0, count: width)
        for (i, limb) in limbs.enumerated() {
            for b in 0..<4 {
                let pos = width - 1 - (i * 4 + b)
                if pos >= 0 { bytes[pos] = UInt8((limb >> UInt32(b * 8)) & 0xFF) }
            }
        }
        return Data(bytes)
    }
}
