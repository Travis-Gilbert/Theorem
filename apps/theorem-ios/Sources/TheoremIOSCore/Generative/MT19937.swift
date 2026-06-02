import Foundation

/// Canonical MT19937 (Matsumoto & Nishimura, 1998), the `init_genrand` seeding
/// variant. Pure and deterministic: the same seed yields the same uint32
/// sequence on every run and every device. Used to seed the hex-blueprint
/// substrate watermark (addendum D3) so each scene prints a stable, reproducible
/// fingerprint, and so a future canonical project seed can be matched bit-for-bit.
///
/// Known-answer (verified against the reference C implementation):
///   seed 5489 -> 3499211612, 581869302, 3890346734
public struct MT19937 {
    private static let n = 624
    private static let m = 397
    private static let matrixA: UInt32 = 0x9908_b0df
    private static let upperMask: UInt32 = 0x8000_0000   // most significant w-r bits
    private static let lowerMask: UInt32 = 0x7fff_ffff   // least significant r bits

    private var mt: [UInt32]
    private var mti: Int

    public init(seed: UInt32) {
        mt = [UInt32](repeating: 0, count: MT19937.n)
        mt[0] = seed
        for i in 1..<MT19937.n {
            let prev = mt[i - 1] ^ (mt[i - 1] >> 30)
            mt[i] = 1812433253 &* prev &+ UInt32(i)
        }
        mti = MT19937.n
    }

    /// Next 32-bit output (tempered).
    public mutating func nextUInt32() -> UInt32 {
        if mti >= MT19937.n {
            generateBlock()
            mti = 0
        }
        var y = mt[mti]
        mti += 1
        y ^= y >> 11
        y ^= (y << 7) & 0x9d2c_5680
        y ^= (y << 15) & 0xefc6_0000
        y ^= y >> 18
        return y
    }

    /// A double in [0, 1) with 32 bits of resolution.
    public mutating func nextDouble() -> Double {
        Double(nextUInt32()) / 4_294_967_296.0   // / 2^32
    }

    private mutating func generateBlock() {
        let n = MT19937.n, m = MT19937.m
        func twist(_ a: UInt32, _ b: UInt32) -> UInt32 {
            let y = (a & MT19937.upperMask) | (b & MT19937.lowerMask)
            return (y >> 1) ^ ((y & 1) == 0 ? 0 : MT19937.matrixA)
        }
        for kk in 0..<(n - m) {
            mt[kk] = mt[kk + m] ^ twist(mt[kk], mt[kk + 1])
        }
        for kk in (n - m)..<(n - 1) {
            mt[kk] = mt[kk + (m - n)] ^ twist(mt[kk], mt[kk + 1])
        }
        mt[n - 1] = mt[m - 1] ^ twist(mt[n - 1], mt[0])
    }

    /// djb2 hash of a string into a 32-bit seed. Used to derive a per-scene
    /// fingerprint (stable for the same scene, different across scenes).
    public static func seed(from string: String) -> UInt32 {
        var hash: UInt32 = 5381
        for byte in string.utf8 {
            hash = (hash << 5) &+ hash &+ UInt32(byte)   // hash * 33 + c
        }
        return hash
    }
}
