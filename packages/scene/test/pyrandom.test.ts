import { test } from "node:test";
import assert from "node:assert/strict";
import { PyRandom } from "../src/pyrandom.ts";

// Expected values from CPython 3 (`random.Random(4)` / `random.Random(20261104)`), see the generator in the commit.
const expected = [[0.23604808973743452, 0.1031660342307158, 0.396058242610681, 0.15497227080241027, 0.06651509567958991], [931.1137101395524, 1292.5685301614033, 1210.3166460470661], [0, 0, 40, 40, 10, 10, 0, 0, 10, 0], [1, 3, 3, 2, 2, 3, 3, 3, 1, 3], ["ramen", "izakaya", "shop", "izakaya", "shop", "karaoke", "karaoke", "izakaya", "shop", "karaoke"], [0, 6, 3, 1, 2, 5, 4], [9, 4], [3531, 3697, 1321, 1910, 2499], [2, 0, 0, 0, 3, 2, 4, 4], [0.7406898336993163, 0.20904628428050342, 0.3708134777752635]];

test("PyRandom reproduces CPython's random.Random sequences", () => {
  const r = new PyRandom(4);
  const got: unknown[] = [];
  got.push(Array.from({ length: 5 }, () => r.random()));
  got.push(Array.from({ length: 3 }, () => r.uniform(650, 1350)));
  got.push(Array.from({ length: 10 }, () => r.choice([0, 0, 10, 25, 40])));
  got.push(Array.from({ length: 10 }, () => r.randint(1, 4)));
  got.push(Array.from({ length: 10 }, () => r.choices(["shop", "izakaya", "ramen", "shutter", "karaoke"], [4, 3, 2, 1, 2])));
  const x = [0, 1, 2, 3, 4, 5, 6];
  r.shuffle(x);
  got.push(x);
  got.push(r.sample([0, 1, 2, 3, 4, 5, 6, 7, 8, 9], 2));
  got.push(Array.from({ length: 5 }, () => r.getrandbits(13)));
  got.push(Array.from({ length: 8 }, () => r.randrange(5)));
  const r2 = new PyRandom(20261104);
  got.push(Array.from({ length: 3 }, () => r2.random()));
  assert.deepEqual(got, expected);
});
