#!/usr/bin/env python3
"""Train the conductor's tiny brain (32 -> 32 -> 16 -> 30 MLP) with numpy.

The training data are synthetic episodes: band / system feature sequences that
walk through the arrangement states (chilling, awakening, alive, subsiding) with
clutter, stubborn players, quotes, spikes, chaos, likes and so on, labelled by the
heuristic conductor plus the emotional rules in src/mood.rs (mirrored below). The
network is a smooth, compact distillation of those rules; it is not a language
model and it never sees your data. Run:

    python3 tools/train_conductor.py            # writes assets/conductor.bin

Outputs: 16 mood logits, 6 intervention logits, 8 speech-intent logits.
"""
import struct, sys, os
import numpy as np

NF, H1, H2, NM, NA, NI = 32, 32, 16, 16, 6, 8
NO = NM + NA + NI
rng = np.random.default_rng(7)

# feature indices (src/mood.rs `f`)
(ENERGY, AGIT, CLUTTER, DENS, OVERLAP, STEP, LOUD, CHILL, AWAKEN, ALIVE, SUBSIDE, CHAOS, BREAK, QUOTE, MINOR, TEMPO,
 CPU, INPUT, NET, DISK, SPIKE, STUBBORN, SINCE, ACTIVE, SURGE, BUILD, LIKE, RELAX, STRICT, CADENCE, TENSION, ONE) = range(32)


def rule_scores(x):
    cl = x[:, CLUTTER]
    ncl = np.maximum(1 - cl, 0)
    s = np.zeros((len(x), 16), np.float32)
    s[:, 0] = 0.6 * ncl * (x[:, AWAKEN] + x[:, ALIVE]) + 0.3 * x[:, CADENCE] + 0.15
    s[:, 1] = 1.4 * x[:, LIKE] + 0.4 * x[:, BUILD] + 0.3 * x[:, CADENCE] * x[:, ENERGY]
    s[:, 2] = 0.5 * x[:, CADENCE] * ncl + 0.35 * x[:, ENERGY] * ncl * x[:, ALIVE]
    s[:, 3] = 0.45 * x[:, SPIKE] * (1 - x[:, CHAOS]) + 0.25 * x[:, INPUT] + 0.15 * x[:, NET]
    s[:, 4] = 0.5 * x[:, ENERGY] * ncl * x[:, ALIVE] + 0.4 * x[:, TENSION] * ncl + 0.35 * x[:, SURGE]
    s[:, 5] = 1.1 * x[:, QUOTE]
    s[:, 6] = 0.45 * x[:, ALIVE] + 0.3 * x[:, AGIT] * ncl + 0.1
    s[:, 7] = 0.5 * (1 - x[:, ENERGY]) * x[:, AGIT] + 0.3 * x[:, INPUT] * x[:, CHILL]
    s[:, 8] = 0.9 * x[:, STUBBORN] + 0.6 * np.maximum(cl - 0.6, 0)
    s[:, 9] = 0.75 * cl + 0.3 * x[:, STEP]
    s[:, 10] = 0.7 * (1 - x[:, ENERGY]) * (1 - x[:, AGIT]) * (x[:, AWAKEN] + x[:, SUBSIDE]) * x[:, SINCE]
    s[:, 11] = 0.8 * x[:, CHILL] * (1 - x[:, AGIT]) * (1 - x[:, INPUT])
    s[:, 12] = 0.95 * x[:, CHAOS] + 0.3 * x[:, CPU] * x[:, AGIT]
    s[:, 13] = 1.1 * x[:, SPIKE]
    s[:, 14] = 0.5 * x[:, CHILL] * (1 - x[:, MINOR]) + 0.3 * x[:, SUBSIDE] * (1 - x[:, MINOR])
    s[:, 15] = 0.5 * x[:, MINOR] * (1 - x[:, ENERGY]) + 0.25 * x[:, SUBSIDE] * x[:, MINOR]
    return s


def softmax(z, gain=1.0):
    z = (z - z.max(1, keepdims=True)) * gain
    e = np.exp(z)
    return e / e.sum(1, keepdims=True)


def rule_action(x):
    strict = 1 + x[:, STRICT]
    cl = x[:, CLUTTER] * strict
    a = np.zeros(len(x), np.int64)
    build = (x[:, ENERGY] > 0.6) & (x[:, AGIT] > 0.6) & (x[:, TENSION] < 0.4) & (x[:, SURGE] < 0.5)
    a[build] = 5
    inv = (cl < 0.08) & (x[:, ACTIVE] < 0.5) & (x[:, CHILL] < 0.5)
    a[inv] = 4
    a[cl > 0.3] = 1
    a[(cl > 0.6) & (x[:, STUBBORN] > 0.3)] = 2
    a[(x[:, SURGE] < 0.5) & (cl > 0.87)] = 3
    return a


INTENT = np.array([1, 1, 1, 4, 6, 5, 0, 3, 2, 2, 3, 7, 7, 6, 7, 7])


def episode(T):
    """one synthetic run: a state walk with correlated band / machine features"""
    x = np.zeros((T, NF), np.float32)
    st = 0
    e = rng.uniform(0, 0.3)
    ag = rng.uniform(0, 0.3)
    style_relax = float(rng.random() < 0.1)
    strict = rng.uniform(-0.3, 0.6)
    minor = float(rng.random() < 0.5)
    tempo = rng.uniform(0.1, 0.8)
    since = 0.0
    like = 0.0
    quote = 0
    surge = 0
    stub = 0.0
    cl = 0.1
    for t in range(T):
        # arrangement state machine
        if rng.random() < 0.04:
            st = (st + 1) % 4
        target_e = [0.15, 0.45, 0.8, 0.35][st]
        e += (target_e - e) * 0.15 + rng.normal(0, 0.05)
        e = float(np.clip(e, 0, 1))
        ag += (rng.uniform(0, 1) * (0.9 if st == 2 else 0.5) - ag) * 0.1 + rng.normal(0, 0.05)
        ag = float(np.clip(ag, 0, 1))
        cl += (rng.uniform(0, 1.2) * e - cl) * 0.2 + rng.normal(0, 0.06)
        cl = float(np.clip(cl, 0, 1.5))
        stub = max(0.0, stub * 0.85 + (rng.random() < 0.05 * cl) * rng.uniform(0.3, 1.0))
        quote = quote - 1 if quote > 0 else (rng.integers(6, 16) if rng.random() < 0.02 else 0)
        surge = surge - 1 if surge > 0 else (rng.integers(8, 16) if rng.random() < 0.01 and st == 2 else 0)
        like = like * 0.7 + (rng.random() < 0.01) * 1.0
        since = min(1.0, since + 0.05) if rng.random() > 0.1 else 0.0
        spike = float(rng.random() < 0.04) * rng.uniform(0.4, 1)
        row = x[t]
        row[ENERGY], row[AGIT], row[CLUTTER] = e, ag, min(cl, 1.0)
        row[DENS] = np.clip(cl * 0.7 + rng.normal(0, 0.1), 0, 1)
        row[OVERLAP] = np.clip(cl * 0.5 + rng.normal(0, 0.1), 0, 1)
        row[STEP] = np.clip(cl * 0.6 + rng.normal(0, 0.15), 0, 1)
        row[LOUD] = np.clip(0.3 + 0.6 * e + rng.normal(0, 0.05), 0, 1)
        row[CHILL + st] = 1
        row[CHAOS] = float(ag > 0.85 and rng.random() < 0.5)
        row[BREAK] = float(rng.random() < 0.05)
        row[QUOTE] = float(quote > 0)
        row[MINOR] = minor
        row[TEMPO] = np.clip(tempo + 0.3 * e + rng.normal(0, 0.02), 0, 1)
        row[CPU] = np.clip(ag * 0.8 + rng.normal(0, 0.1), 0, 1)
        row[INPUT] = np.clip(rng.uniform(0, 1) * ag, 0, 1)
        row[NET] = np.clip(rng.uniform(0, 0.6) * e, 0, 1)
        row[DISK] = np.clip(rng.uniform(0, 0.6) * e, 0, 1)
        row[SPIKE] = spike
        row[STUBBORN] = min(stub, 1.0)
        row[SINCE] = since
        row[ACTIVE] = np.clip(0.2 + 0.7 * e + rng.normal(0, 0.1), 0, 1)
        row[SURGE] = float(surge > 0)
        row[BUILD] = float(style_relax and st == 2 and rng.random() < 0.3)
        row[LIKE] = like
        row[RELAX] = style_relax
        row[STRICT] = strict
        row[CADENCE] = float(rng.random() < 0.15)
        row[TENSION] = np.clip(rng.uniform(0, 1), 0, 1)
        row[ONE] = 1
    return x


def make(n_ep=600, T=200):
    X = np.concatenate([episode(T) for _ in range(n_ep)])
    M = softmax(rule_scores(X), 4.0)
    A = rule_action(X)
    I = INTENT[M.argmax(1)]
    return X, M, A, I


def init(a, b):
    return rng.normal(0, 1 / np.sqrt(a), (a, b)).astype(np.float32), np.zeros(b, np.float32)


def forward(p, X):
    h1 = np.tanh(X @ p[0] + p[1])
    h2 = np.tanh(h1 @ p[2] + p[3])
    o = h2 @ p[4] + p[5]
    return h1, h2, o


def train():
    X, M, A, I = make()
    n = len(X)
    print(f"{n} synthetic bars")
    p = [*init(NF, H1), *init(H1, H2), *init(H2, NO)]
    m = [np.zeros_like(q) for q in p]
    v = [np.zeros_like(q) for q in p]
    lr, b1, b2 = 3e-3, 0.9, 0.999
    step = 0
    for ep in range(30):
        idx = rng.permutation(n)
        for k in range(0, n, 256):
            j = idx[k:k + 256]
            x, mt, at, it = X[j], M[j], A[j], I[j]
            h1, h2, o = forward(p, x)
            pm = softmax(o[:, :NM])
            pa = softmax(o[:, NM:NM + NA])
            pi = softmax(o[:, NM + NA:])
            g = np.zeros_like(o)
            g[:, :NM] = pm - mt
            ga = pa.copy()
            ga[np.arange(len(j)), at] -= 1
            g[:, NM:NM + NA] = ga
            gi = pi.copy()
            gi[np.arange(len(j)), it] -= 1
            g[:, NM + NA:] = 0.5 * gi
            g /= len(j)
            grads = [None] * 6
            grads[4] = h2.T @ g
            grads[5] = g.sum(0)
            d2 = (g @ p[4].T) * (1 - h2 ** 2)
            grads[2] = h1.T @ d2
            grads[3] = d2.sum(0)
            d1 = (d2 @ p[2].T) * (1 - h1 ** 2)
            grads[0] = x.T @ d1
            grads[1] = d1.sum(0)
            step += 1
            for q in range(6):
                m[q] = b1 * m[q] + (1 - b1) * grads[q]
                v[q] = b2 * v[q] + (1 - b2) * grads[q] ** 2
                mh = m[q] / (1 - b1 ** step)
                vh = v[q] / (1 - b2 ** step)
                p[q] -= lr * mh / (np.sqrt(vh) + 1e-8)
        if ep % 5 == 4 or ep == 29:
            _, _, o = forward(p, X[:20000])
            acc_m = (o[:, :NM].argmax(1) == M[:20000].argmax(1)).mean()
            acc_a = (o[:, NM:NM + NA].argmax(1) == A[:20000]).mean()
            acc_i = (o[:, NM + NA:].argmax(1) == I[:20000]).mean()
            print(f"epoch {ep + 1}: mood top-1 {acc_m:.3f}, action {acc_a:.3f}, intent {acc_i:.3f}")
    # held-out check
    Xt, Mt, At, It = make(60, 200)
    _, _, o = forward(p, Xt)
    print(f"held-out: mood top-1 {(o[:, :NM].argmax(1) == Mt.argmax(1)).mean():.3f}, action {(o[:, NM:NM + NA].argmax(1) == At).mean():.3f}, intent {(o[:, NM + NA:].argmax(1) == It).mean():.3f}")
    return p


def save(p, path):
    with open(path, "wb") as f:
        f.write(b"SJB1")
        f.write(struct.pack("<6I", NF, H1, H2, NM, NA, NI))
        for q in p:
            f.write(q.astype("<f4").tobytes())
    print(f"wrote {path} ({os.path.getsize(path)} bytes)")


if __name__ == "__main__":
    out = sys.argv[1] if len(sys.argv) > 1 else os.path.join(os.path.dirname(__file__), "..", "assets", "conductor.bin")
    save(train(), out)
