// The moon over Tokyo: where it stands, how much of it is lit, and the light it sends, at a moment (UTC ms). Schlyter's
// method (mean orbital elements plus the main perturbations, within a few arcminutes): position from the moon's and the
// sun's mean elements, then horizontal coordinates for the observer. Light: the moon's magnitude by phase angle (Allen:
// -12.73 + 0.026|a| + 4e-9 a^4, at its mean distance) against the sun's -26.74, so its illuminance as a fraction of the
// sun's, both above the atmosphere; then the air between, with the sky's own constants (renderer/src/sky.rs Earth).
const RAD = Math.PI / 180;
const rev = (x: number) => ((x % 360) + 360) % 360;
const sin = (deg: number) => Math.sin(deg * RAD), cos = (deg: number) => Math.cos(deg * RAD);

export type Moon = {
  /** Degrees above the horizon (topocentric: parallax taken off). */
  elevation: number;
  /** Degrees, as `DirectionalLight.setSunAngles` takes them: from the south (+z) towards the east (+x). */
  azimuth: number;
  /** The fraction of its disc that is lit, 0 (new) to 1 (full). */
  lit: number;
  /** Its illuminance as a fraction of the sun's, both above the atmosphere (full moon at perigee: ~2.5e-6). */
  toSun: number;
};

/** The moon at `utcMs` seen from (lat, lon) degrees; by default Tokyo Tower. */
export function moonAt(utcMs: number, lat = 35.6586, lon = 139.7454): Moon {
  const d = utcMs / 86400000 + 2440587.5 - 2451543.5; // days from 2000 Jan 0.0 UT
  const ut = (((utcMs / 3600000) % 24) + 24) % 24;
  // the sun: its true longitude (for the phase) and mean longitude (for sidereal time and the perturbations)
  const ws = 282.9404 + 4.70935e-5 * d, es = 0.016709 - 1.151e-9 * d, Ms = rev(356.047 + 0.9856002585 * d);
  const Es = Ms + (es / RAD) * sin(Ms) * (1 + es * cos(Ms));
  const sunLon = rev(Math.atan2(Math.sqrt(1 - es * es) * sin(Es), cos(Es) - es) / RAD + ws);
  const Ls = rev(Ms + ws);
  // the moon's orbit: ascending node, inclination, argument of perigee, mean distance (Earth radii), eccentricity, mean anomaly
  const N = rev(125.1228 - 0.0529538083 * d), i = 5.1454, w = rev(318.0634 + 0.1643573223 * d), a = 60.2666, e = 0.0549;
  const M = rev(115.3654 + 13.0649929509 * d);
  let E = M + (e / RAD) * sin(M) * (1 + e * cos(M));
  for (let k = 0; k < 5; k++) E -= (E - (e / RAD) * sin(E) - M) / (1 - e * cos(E));
  const xv = a * (cos(E) - e), yv = a * Math.sqrt(1 - e * e) * sin(E);
  const vw = Math.atan2(yv, xv) / RAD + w;
  let r = Math.hypot(xv, yv);
  const xh = r * (cos(N) * cos(vw) - sin(N) * sin(vw) * cos(i));
  const yh = r * (sin(N) * cos(vw) + cos(N) * sin(vw) * cos(i));
  const zh = r * sin(vw) * sin(i);
  let mLon = Math.atan2(yh, xh) / RAD, mLat = Math.atan2(zh, Math.hypot(xh, yh)) / RAD;
  // the main perturbations (degrees; Earth radii)
  const Lm = rev(M + w + N), D = rev(Lm - Ls), F = rev(Lm - N);
  mLon += -1.274 * sin(M - 2 * D) + 0.658 * sin(2 * D) - 0.186 * sin(Ms) - 0.059 * sin(2 * M - 2 * D) - 0.057 * sin(M - 2 * D + Ms) + 0.053 * sin(M + 2 * D) + 0.046 * sin(2 * D - Ms) + 0.041 * sin(M - Ms) - 0.035 * sin(D) - 0.031 * sin(M + Ms) - 0.015 * sin(2 * F - 2 * D) + 0.011 * sin(M - 4 * D);
  mLat += -0.173 * sin(F - 2 * D) - 0.055 * sin(M - F - 2 * D) - 0.046 * sin(M + F - 2 * D) + 0.033 * sin(F + 2 * D) + 0.017 * sin(2 * M + F);
  r += -0.58 * cos(M - 2 * D) - 0.46 * cos(2 * D);
  // ecliptic -> equatorial -> horizontal
  const ecl = 23.4393 - 3.563e-7 * d;
  const xe = cos(mLon) * cos(mLat), ye = sin(mLon) * cos(mLat) * cos(ecl) - sin(mLat) * sin(ecl), ze = sin(mLon) * cos(mLat) * sin(ecl) + sin(mLat) * cos(ecl);
  const ra = Math.atan2(ye, xe) / RAD, dec = Math.atan2(ze, Math.hypot(xe, ye)) / RAD;
  const ha = rev(Ls + 180 + ut * 15 + lon - ra);
  const x = cos(ha) * cos(dec), y = sin(ha) * cos(dec), z = sin(dec);
  const xHor = x * sin(lat) - z * cos(lat), zHor = x * cos(lat) + z * sin(lat);
  let elevation = Math.asin(zHor) / RAD;
  elevation -= (Math.asin(1 / r) / RAD) * cos(elevation);
  // azimuth from the north, clockwise (east 90), then from the south towards the east
  const fromNorth = rev(Math.atan2(y, xHor) / RAD + 180);
  // phase angle (sun - moon - earth), the lit fraction and the magnitude
  const phase = 180 - Math.acos(cos(sunLon - mLon) * cos(mLat)) / RAD;
  const mag = -12.73 + 0.026 * Math.abs(phase) + 4e-9 * phase ** 4;
  return { elevation, azimuth: rev(180 - fromNorth), lit: (1 + cos(phase)) / 2, toSun: 10 ** (-0.4 * (mag + 26.74)) * (a / r) ** 2 };
}

/**
 * Direct sunlight's (or moonlight's) transmittance through the air at `elevation` degrees, RGB: Rayleigh, Mie at
 * `mieScale` and ozone with renderer/src/sky.rs's Earth (scale heights 8 km and 1.2 km; ozone a 30 km tent about 25 km),
 * over Kasten and Young's air mass. Zero below the horizon.
 */
export function airTransmittance(elevation: number, mieScale = 1): [number, number, number] {
  if (elevation <= 0) return [0, 0, 0];
  const mass = 1 / (sin(elevation) + 0.50572 * (elevation + 6.07995) ** -1.6364);
  const rayleigh = [5.802e-3, 13.558e-3, 33.1e-3].map((s) => s * 8); // per km x 8 km
  const mie = (3.996e-3 * mieScale + 0.444e-3) * 1.2;
  const ozone = [0.65e-3, 1.881e-3, 0.085e-3].map((s) => s * 15);
  return [0, 1, 2].map((k) => Math.exp(-mass * (rayleigh[k] + mie + ozone[k]))) as [number, number, number];
}
