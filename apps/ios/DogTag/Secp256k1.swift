import Foundation

/// secp256k1 address derivation (priv → uncompressed pubkey → keccak256[12:]).
///
/// Implemented with Foundation-free big-integer modular arithmetic (a small `BigUInt` over
/// [UInt32] limbs) so the Ethereum address is computed EXACTLY (no third-party secp256k1 dep is
/// available in this project). Jacobian-coordinate scalar multiply over the curve y² = x³ + 7.
enum Secp256k1 {
    // Field prime p and group order n.
    static let p = BigUInt(hex: "fffffffffffffffffffffffffffffffffffffffffffffffffffffffefffffc2f")
    static let n = BigUInt(hex: "fffffffffffffffffffffffffffffffebaaedce6af48a03bbfd25e8cd0364141")
    static let gx = BigUInt(hex: "79be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798")
    static let gy = BigUInt(hex: "483ada7726a3c4655da4fbfc0e1108a8fd17b448a68554199c47d08ffb10d4b8")

    static func address(fromPriv priv: Data) -> String {
        var d = BigUInt(data: priv).mod(n)
        if d.isZero { d = BigUInt(1) }
        let (x, y) = scalarMulBase(d)
        var pub = Data()
        pub.append(x.toData(32))
        pub.append(y.toData(32))
        let hash = Keccak256.digest(pub)
        let addr = hash.suffix(20)
        return "0x" + addr.map { String(format: "%02x", $0) }.joined()
    }

    // ---- Jacobian point ops ------------------------------------------------------------------
    private struct Jac { var x: BigUInt; var y: BigUInt; var z: BigUInt; var inf: Bool }

    private static func double(_ pt: Jac) -> Jac {
        if pt.inf || pt.y.isZero { return Jac(x: BigUInt(1), y: BigUInt(1), z: BigUInt(0), inf: true) }
        // standard a=0 doubling
        let a = mulm(pt.x, pt.x)               // x^2
        let b = mulm(pt.y, pt.y)               // y^2
        let c = mulm(b, b)                     // y^4
        var d = mulm(addm(pt.x, b), addm(pt.x, b)) // (x+y^2)^2
        d = subm(d, a); d = subm(d, c); d = addm(d, d) // 2*((x+y^2)^2 - x^2 - y^4)
        let e = addm(addm(a, a), a)            // 3*x^2
        let f = mulm(e, e)                     // e^2
        var x3 = subm(f, addm(d, d))
        var y3 = subm(mulm(e, subm(d, x3)), mulm(BigUInt(8), c))
        let z3 = mulm(addm(pt.y, pt.y), pt.z)
        x3 = x3.mod(p); y3 = y3.mod(p)
        return Jac(x: x3, y: y3, z: z3, inf: false)
    }

    private static func add(_ p1: Jac, _ p2: Jac) -> Jac {
        if p1.inf { return p2 }
        if p2.inf { return p1 }
        let z1z1 = mulm(p1.z, p1.z)
        let z2z2 = mulm(p2.z, p2.z)
        let u1 = mulm(p1.x, z2z2)
        let u2 = mulm(p2.x, z1z1)
        let s1 = mulm(mulm(p1.y, p2.z), z2z2)
        let s2 = mulm(mulm(p2.y, p1.z), z1z1)
        if u1.eq(u2) {
            if !s1.eq(s2) { return Jac(x: BigUInt(1), y: BigUInt(1), z: BigUInt(0), inf: true) }
            return double(p1)
        }
        let h = subm(u2, u1)
        let i = mulm(addm(h, h), addm(h, h))
        let j = mulm(h, i)
        let r = addm(subm(s2, s1), subm(s2, s1))
        let v = mulm(u1, i)
        var x3 = subm(subm(mulm(r, r), j), addm(v, v))
        var y3 = subm(mulm(r, subm(v, x3)), mulm(addm(s1, s1), j))
        var z3 = mulm(mulm(subm(mulm(addm(p1.z, p2.z), addm(p1.z, p2.z)), addm(z1z1, z2z2)), h), BigUInt(1))
        x3 = x3.mod(p); y3 = y3.mod(p); z3 = z3.mod(p)
        return Jac(x: x3, y: y3, z: z3, inf: false)
    }

    private static func scalarMulBase(_ k: BigUInt) -> (BigUInt, BigUInt) {
        var result = Jac(x: BigUInt(1), y: BigUInt(1), z: BigUInt(0), inf: true)
        var addend = Jac(x: gx, y: gy, z: BigUInt(1), inf: false)
        let bits = k.bitLength
        for i in 0..<bits {
            if k.bit(i) { result = add(result, addend) }
            addend = double(addend)
        }
        return toAffine(result)
    }

    private static func toAffine(_ pt: Jac) -> (BigUInt, BigUInt) {
        if pt.inf { return (BigUInt(0), BigUInt(0)) }
        let zinv = pt.z.modInverse(p)
        let zinv2 = mulm(zinv, zinv)
        let zinv3 = mulm(zinv2, zinv)
        return (mulm(pt.x, zinv2), mulm(pt.y, zinv3))
    }

    // mod-p helpers
    private static func mulm(_ a: BigUInt, _ b: BigUInt) -> BigUInt { a.mul(b).mod(p) }
    private static func addm(_ a: BigUInt, _ b: BigUInt) -> BigUInt { a.add(b).mod(p) }
    private static func subm(_ a: BigUInt, _ b: BigUInt) -> BigUInt {
        let am = a.mod(p), bm = b.mod(p)
        return am.cmp(bm) >= 0 ? am.sub(bm) : am.add(p).sub(bm)
    }
}
