import Foundation
import CryptoKit

/// HMAC-SHA256 over CryptoKit, used by RFC-6979 deterministic-nonce generation.
enum HmacSHA256 {
    static func mac(key: Data, data: Data) -> Data {
        let k = SymmetricKey(data: key)
        return Data(HMAC<SHA256>.authenticationCode(for: data, using: k))
    }
}

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

    /**
     Sign a 32-byte digest with the secp256k1 private key → 65-byte Ethereum `r || s || v` (0x.. hex).
     Deterministic nonce (RFC-6979 / HMAC-SHA256), low-S normalised, recovery id 27/28. This is the
     raw EIP-712 digest signature consumed by `ConsentKeyRegistry.bindConsentKeyFor` (digest from the
     Rust FFI `bindConsentKeyDigestHex`).
     */
    static func signDigest(priv: Data, digest: Data) -> String {
        precondition(digest.count == 32, "digest must be 32 bytes")
        var d = BigUInt(data: priv).mod(n)
        if d.isZero { d = BigUInt(1) }
        let e = BigUInt(data: digest).mod(n)

        var counter = 0
        while true {
            let k = rfc6979Nonce(priv: priv, digest: digest, counter: counter).mod(n)
            counter += 1
            if k.isZero { continue }
            let (rx, _) = scalarMulBase(k)
            let r = rx.mod(n)
            if r.isZero { continue }
            // s = k^-1 * (e + r*d) mod n
            let kInv = k.modInverse(n)
            let rd = r.mul(d).mod(n)
            let s0 = kInv.mul(e.add(rd).mod(n)).mod(n)
            if s0.isZero { continue }
            // Low-S normalisation (EIP-2): if s > n/2, replace with n - s. The recovery id is then
            // computed against this normalized `s` directly (see findRecId below), so NO separate
            // parity flip is tracked — mirrors Android Wallet.kt:272-284. Tracking a `flip` AND
            // recovering against the normalized s double-counts the parity → wrong `v` ~50% of the
            // time → on-chain ECDSA.recover returns a different address → "bad sig".
            let halfN = n.sub(BigUInt(1)).shiftRight1() // (n-1)/2; n is odd so floor(n/2)
            var s = s0
            if s.cmp(halfN) > 0 { s = n.sub(s) }
            // Recover the public key to determine v against our own pubkey (the ground truth), using
            // the ALREADY-NORMALIZED `s`. v = recId + 27 (no separate flip).
            let q = scalarMulBase(d)
            let recId = findRecId(r: r, s: s, e: e, expectedQ: q) ?? 0
            let v = 27 + recId
            let rb = r.toData(32).map { String(format: "%02x", $0) }.joined()
            let sb = s.toData(32).map { String(format: "%02x", $0) }.joined()
            return "0x" + rb + sb + String(format: "%02x", v)
        }
    }

    /// Recover the recovery id (0/1) for signature `(r, s)` over digest-scalar `e` such that the
    /// recovered public key equals the signer's own `expectedQ` (affine). Mirrors Android
    /// `Wallet.kt:findRecId`: for each candidate parity, decompress R = (r, yBit), then compute the
    /// candidate pubkey Q = r⁻¹·(s·R − e·G) and compare. `s` is passed already low-S-normalized, so
    /// the returned recId is correct for the returned signature — the caller must NOT flip it again.
    private static func findRecId(
        r: BigUInt, s: BigUInt, e: BigUInt, expectedQ: (BigUInt, BigUInt)
    ) -> Int? {
        let rInv = r.modInverse(n)
        for recId in 0...1 {
            // R = (r, y) with y-parity == recId (r < n, so the x-coordinate is r itself).
            guard let rPoint = decompress(x: r, yBit: recId) else { continue }
            // Q = r⁻¹ · (s·R + (n - e)·G)  ==  r⁻¹ · (s·R − e·G).
            let srP = jacScalarMul(s.mod(n), rPoint)
            let negeG = scalarMulBaseJac(n.sub(e.mod(n)).mod(n))
            let sum = add(srP, negeG)
            let qCand = jacScalarMul(rInv, sum)
            let (qx, qy) = toAffine(qCand)
            if qx.eq(expectedQ.0) && qy.eq(expectedQ.1) { return recId }
        }
        return nil
    }

    /// Decompress an affine point from its x-coordinate and y-parity on y² = x³ + 7 (mod p).
    /// Returns nil if x is not on the curve.
    private static func decompress(x: BigUInt, yBit: Int) -> Jac? {
        // rhs = x³ + 7 (mod p)
        let x3 = mulm(mulm(x, x), x)
        let rhs = addm(x3, BigUInt(7))
        // y = rhs^((p+1)/4) mod p  (p ≡ 3 mod 4).
        let y = rhs.modPow(p.add(BigUInt(1)).shiftRight1().shiftRight1(), p)
        // Verify y² == rhs (x on curve).
        if !mulm(y, y).eq(rhs.mod(p)) { return nil }
        let yParity = Int(y.limbs.first ?? 0) & 1
        let yFinal = (yParity == yBit) ? y : p.sub(y)
        return Jac(x: x.mod(p), y: yFinal, z: BigUInt(1), inf: false)
    }

    /// RFC-6979 deterministic nonce k (HMAC-SHA256, no extra entropy). `counter` re-runs the generator
    /// loop if the first candidate is rejected.
    private static func rfc6979Nonce(priv: Data, digest: Data, counter: Int) -> BigUInt {
        let qlen = 256
        var v = Data(repeating: 0x01, count: 32)
        var kk = Data(repeating: 0x00, count: 32)
        let x = BigUInt(data: priv).mod(n).toData(32)
        let h1 = digest // already 32 bytes, reduced is fine for bits2octets at this size
        kk = HmacSHA256.mac(key: kk, data: v + Data([0x00]) + x + h1)
        v = HmacSHA256.mac(key: kk, data: v)
        kk = HmacSHA256.mac(key: kk, data: v + Data([0x01]) + x + h1)
        v = HmacSHA256.mac(key: kk, data: v)
        var iters = 0
        while true {
            var t = Data()
            while t.count < (qlen + 7) / 8 {
                v = HmacSHA256.mac(key: kk, data: v)
                t.append(v)
            }
            let cand = BigUInt(data: t.prefix(32))
            if iters >= counter && !cand.isZero && cand.cmp(n) < 0 {
                return cand
            }
            iters += 1
            kk = HmacSHA256.mac(key: kk, data: v + Data([0x00]))
            v = HmacSHA256.mac(key: kk, data: v)
        }
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
        toAffine(scalarMulBaseJac(k))
    }

    /// k·G in Jacobian coordinates.
    private static func scalarMulBaseJac(_ k: BigUInt) -> Jac {
        jacScalarMul(k, Jac(x: gx, y: gy, z: BigUInt(1), inf: false))
    }

    /// k·P for an arbitrary Jacobian point P (double-and-add).
    private static func jacScalarMul(_ k: BigUInt, _ base: Jac) -> Jac {
        var result = Jac(x: BigUInt(1), y: BigUInt(1), z: BigUInt(0), inf: true)
        var addend = base
        let bits = k.bitLength
        for i in 0..<bits {
            if k.bit(i) { result = add(result, addend) }
            addend = double(addend)
        }
        return result
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
