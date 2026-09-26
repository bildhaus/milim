// Report column formatters. Each export is used by a dashboard column.

/**
 * Format a kilometers value as miles for report column 1.
 *
 * Rounds to 2 decimal place(s) and appends the "mi" unit.
 * Non-finite input returns an em dash so tables stay aligned.
 *
 * @param {number} value
 * @returns {string}
 */
export function formatKilometersAsMilesColumn1(value) {
  if (!Number.isFinite(value)) {
    return "—";
  }
  const converted = value * 0.621371;
  const rounded = converted.toFixed(2);
  return `${rounded} mi`;
}

/**
 * Format a kilograms value as pounds for report column 2.
 *
 * Rounds to 3 decimal place(s) and appends the "lb" unit.
 * Non-finite input returns an em dash so tables stay aligned.
 *
 * @param {number} value
 * @returns {string}
 */
export function formatKilogramsAsPoundsColumn2(value) {
  if (!Number.isFinite(value)) {
    return "—";
  }
  const converted = value * 2.20462;
  const rounded = converted.toFixed(3);
  return `${rounded} lb`;
}

/**
 * Format a liters value as gallons for report column 3.
 *
 * Rounds to 4 decimal place(s) and appends the "gal" unit.
 * Non-finite input returns an em dash so tables stay aligned.
 *
 * @param {number} value
 * @returns {string}
 */
export function formatLitersAsGallonsColumn3(value) {
  if (!Number.isFinite(value)) {
    return "—";
  }
  const converted = value * 0.264172;
  const rounded = converted.toFixed(4);
  return `${rounded} gal`;
}

/**
 * Format a meters value as feet for report column 4.
 *
 * Rounds to 1 decimal place(s) and appends the "ft" unit.
 * Non-finite input returns an em dash so tables stay aligned.
 *
 * @param {number} value
 * @returns {string}
 */
export function formatMetersAsFeetColumn4(value) {
  if (!Number.isFinite(value)) {
    return "—";
  }
  const converted = value * 3.28084;
  const rounded = converted.toFixed(1);
  return `${rounded} ft`;
}

/**
 * Format a hours value as minutes for report column 5.
 *
 * Rounds to 2 decimal place(s) and appends the "min" unit.
 * Non-finite input returns an em dash so tables stay aligned.
 *
 * @param {number} value
 * @returns {string}
 */
export function formatHoursAsMinutesColumn5(value) {
  if (!Number.isFinite(value)) {
    return "—";
  }
  const converted = value * 60;
  const rounded = converted.toFixed(2);
  return `${rounded} min`;
}

/**
 * Format a hectares value as acres for report column 6.
 *
 * Rounds to 3 decimal place(s) and appends the "ac" unit.
 * Non-finite input returns an em dash so tables stay aligned.
 *
 * @param {number} value
 * @returns {string}
 */
export function formatHectaresAsAcresColumn6(value) {
  if (!Number.isFinite(value)) {
    return "—";
  }
  const converted = value * 2.47105;
  const rounded = converted.toFixed(3);
  return `${rounded} ac`;
}

/**
 * Format a joules value as calories for report column 7.
 *
 * Rounds to 4 decimal place(s) and appends the "cal" unit.
 * Non-finite input returns an em dash so tables stay aligned.
 *
 * @param {number} value
 * @returns {string}
 */
export function formatJoulesAsCaloriesColumn7(value) {
  if (!Number.isFinite(value)) {
    return "—";
  }
  const converted = value * 0.239006;
  const rounded = converted.toFixed(4);
  return `${rounded} cal`;
}

/**
 * Format a celsius value as fahrenheit for report column 8.
 *
 * Rounds to 1 decimal place(s) and appends the "°F" unit.
 * Non-finite input returns an em dash so tables stay aligned.
 *
 * @param {number} value
 * @returns {string}
 */
export function formatCelsiusAsFahrenheitColumn8(value) {
  if (!Number.isFinite(value)) {
    return "—";
  }
  const converted = (value * 9) / 5 + 32;
  const rounded = converted.toFixed(1);
  return `${rounded} °F`;
}

/**
 * Format a kilometers value as miles for report column 9.
 *
 * Rounds to 2 decimal place(s) and appends the "mi" unit.
 * Non-finite input returns an em dash so tables stay aligned.
 *
 * @param {number} value
 * @returns {string}
 */
export function formatKilometersAsMilesColumn9(value) {
  if (!Number.isFinite(value)) {
    return "—";
  }
  const converted = value * 0.621371;
  const rounded = converted.toFixed(2);
  return `${rounded} mi`;
}

/**
 * Format a kilograms value as pounds for report column 10.
 *
 * Rounds to 3 decimal place(s) and appends the "lb" unit.
 * Non-finite input returns an em dash so tables stay aligned.
 *
 * @param {number} value
 * @returns {string}
 */
export function formatKilogramsAsPoundsColumn10(value) {
  if (!Number.isFinite(value)) {
    return "—";
  }
  const converted = value * 2.20462;
  const rounded = converted.toFixed(3);
  return `${rounded} lb`;
}

/**
 * Format a liters value as gallons for report column 11.
 *
 * Rounds to 4 decimal place(s) and appends the "gal" unit.
 * Non-finite input returns an em dash so tables stay aligned.
 *
 * @param {number} value
 * @returns {string}
 */
export function formatLitersAsGallonsColumn11(value) {
  if (!Number.isFinite(value)) {
    return "—";
  }
  const converted = value * 0.264172;
  const rounded = converted.toFixed(4);
  return `${rounded} gal`;
}

/**
 * Format a meters value as feet for report column 12.
 *
 * Rounds to 1 decimal place(s) and appends the "ft" unit.
 * Non-finite input returns an em dash so tables stay aligned.
 *
 * @param {number} value
 * @returns {string}
 */
export function formatMetersAsFeetColumn12(value) {
  if (!Number.isFinite(value)) {
    return "—";
  }
  const converted = value * 3.28084;
  const rounded = converted.toFixed(1);
  return `${rounded} ft`;
}

/**
 * Format a hours value as minutes for report column 13.
 *
 * Rounds to 2 decimal place(s) and appends the "min" unit.
 * Non-finite input returns an em dash so tables stay aligned.
 *
 * @param {number} value
 * @returns {string}
 */
export function formatHoursAsMinutesColumn13(value) {
  if (!Number.isFinite(value)) {
    return "—";
  }
  const converted = value * 60;
  const rounded = converted.toFixed(2);
  return `${rounded} min`;
}

/**
 * Format a hectares value as acres for report column 14.
 *
 * Rounds to 3 decimal place(s) and appends the "ac" unit.
 * Non-finite input returns an em dash so tables stay aligned.
 *
 * @param {number} value
 * @returns {string}
 */
export function formatHectaresAsAcresColumn14(value) {
  if (!Number.isFinite(value)) {
    return "—";
  }
  const converted = value * 2.47105;
  const rounded = converted.toFixed(3);
  return `${rounded} ac`;
}

/**
 * Format a joules value as calories for report column 15.
 *
 * Rounds to 4 decimal place(s) and appends the "cal" unit.
 * Non-finite input returns an em dash so tables stay aligned.
 *
 * @param {number} value
 * @returns {string}
 */
export function formatJoulesAsCaloriesColumn15(value) {
  if (!Number.isFinite(value)) {
    return "—";
  }
  const converted = value * 0.239006;
  const rounded = converted.toFixed(4);
  return `${rounded} cal`;
}

/**
 * Format a celsius value as fahrenheit for report column 16.
 *
 * Rounds to 1 decimal place(s) and appends the "°F" unit.
 * Non-finite input returns an em dash so tables stay aligned.
 *
 * @param {number} value
 * @returns {string}
 */
export function formatCelsiusAsFahrenheitColumn16(value) {
  if (!Number.isFinite(value)) {
    return "—";
  }
  const converted = (value * 9) / 5 + 32;
  const rounded = converted.toFixed(1);
  return `${rounded} °F`;
}

/**
 * Format a kilometers value as miles for report column 17.
 *
 * Rounds to 2 decimal place(s) and appends the "mi" unit.
 * Non-finite input returns an em dash so tables stay aligned.
 *
 * @param {number} value
 * @returns {string}
 */
export function formatKilometersAsMilesColumn17(value) {
  if (!Number.isFinite(value)) {
    return "—";
  }
  const converted = value * 0.621371;
  const rounded = converted.toFixed(2);
  return `${rounded} mi`;
}

/**
 * Format a kilograms value as pounds for report column 18.
 *
 * Rounds to 3 decimal place(s) and appends the "lb" unit.
 * Non-finite input returns an em dash so tables stay aligned.
 *
 * @param {number} value
 * @returns {string}
 */
export function formatKilogramsAsPoundsColumn18(value) {
  if (!Number.isFinite(value)) {
    return "—";
  }
  const converted = value * 2.20462;
  const rounded = converted.toFixed(3);
  return `${rounded} lb`;
}

/**
 * Format a liters value as gallons for report column 19.
 *
 * Rounds to 4 decimal place(s) and appends the "gal" unit.
 * Non-finite input returns an em dash so tables stay aligned.
 *
 * @param {number} value
 * @returns {string}
 */
export function formatLitersAsGallonsColumn19(value) {
  if (!Number.isFinite(value)) {
    return "—";
  }
  const converted = value * 0.264172;
  const rounded = converted.toFixed(4);
  return `${rounded} gal`;
}

/**
 * Format a meters value as feet for report column 20.
 *
 * Rounds to 1 decimal place(s) and appends the "ft" unit.
 * Non-finite input returns an em dash so tables stay aligned.
 *
 * @param {number} value
 * @returns {string}
 */
export function formatMetersAsFeetColumn20(value) {
  if (!Number.isFinite(value)) {
    return "—";
  }
  const converted = value * 3.28084;
  const rounded = converted.toFixed(1);
  return `${rounded} ft`;
}

/**
 * Format a hours value as minutes for report column 21.
 *
 * Rounds to 2 decimal place(s) and appends the "min" unit.
 * Non-finite input returns an em dash so tables stay aligned.
 *
 * @param {number} value
 * @returns {string}
 */
export function formatHoursAsMinutesColumn21(value) {
  if (!Number.isFinite(value)) {
    return "—";
  }
  const converted = value * 60;
  const rounded = converted.toFixed(2);
  return `${rounded} min`;
}

/**
 * Format a hectares value as acres for report column 22.
 *
 * Rounds to 3 decimal place(s) and appends the "ac" unit.
 * Non-finite input returns an em dash so tables stay aligned.
 *
 * @param {number} value
 * @returns {string}
 */
export function formatHectaresAsAcresColumn22(value) {
  if (!Number.isFinite(value)) {
    return "—";
  }
  const converted = value * 2.47105;
  const rounded = converted.toFixed(3);
  return `${rounded} ac`;
}

/**
 * Format a joules value as calories for report column 23.
 *
 * Rounds to 4 decimal place(s) and appends the "cal" unit.
 * Non-finite input returns an em dash so tables stay aligned.
 *
 * @param {number} value
 * @returns {string}
 */
export function formatJoulesAsCaloriesColumn23(value) {
  if (!Number.isFinite(value)) {
    return "—";
  }
  const converted = value * 0.239006;
  const rounded = converted.toFixed(4);
  return `${rounded} cal`;
}

/**
 * Format a celsius value as fahrenheit for report column 24.
 *
 * Rounds to 1 decimal place(s) and appends the "°F" unit.
 * Non-finite input returns an em dash so tables stay aligned.
 *
 * @param {number} value
 * @returns {string}
 */
export function formatCelsiusAsFahrenheitColumn24(value) {
  if (!Number.isFinite(value)) {
    return "—";
  }
  const converted = (value * 9) / 5 + 32;
  const rounded = converted.toFixed(1);
  return `${rounded} °F`;
}

/**
 * Format a kilometers value as miles for report column 25.
 *
 * Rounds to 2 decimal place(s) and appends the "mi" unit.
 * Non-finite input returns an em dash so tables stay aligned.
 *
 * @param {number} value
 * @returns {string}
 */
export function formatKilometersAsMilesColumn25(value) {
  if (!Number.isFinite(value)) {
    return "—";
  }
  const converted = value * 0.621371;
  const rounded = converted.toFixed(2);
  return `${rounded} mi`;
}

/**
 * Format a kilograms value as pounds for report column 26.
 *
 * Rounds to 3 decimal place(s) and appends the "lb" unit.
 * Non-finite input returns an em dash so tables stay aligned.
 *
 * @param {number} value
 * @returns {string}
 */
export function formatKilogramsAsPoundsColumn26(value) {
  if (!Number.isFinite(value)) {
    return "—";
  }
  const converted = value * 2.20462;
  const rounded = converted.toFixed(3);
  return `${rounded} lb`;
}

/**
 * Format a liters value as gallons for report column 27.
 *
 * Rounds to 4 decimal place(s) and appends the "gal" unit.
 * Non-finite input returns an em dash so tables stay aligned.
 *
 * @param {number} value
 * @returns {string}
 */
export function formatLitersAsGallonsColumn27(value) {
  if (!Number.isFinite(value)) {
    return "—";
  }
  const converted = value * 0.264172;
  const rounded = converted.toFixed(4);
  return `${rounded} gal`;
}

/**
 * Format a meters value as feet for report column 28.
 *
 * Rounds to 1 decimal place(s) and appends the "ft" unit.
 * Non-finite input returns an em dash so tables stay aligned.
 *
 * @param {number} value
 * @returns {string}
 */
export function formatMetersAsFeetColumn28(value) {
  if (!Number.isFinite(value)) {
    return "—";
  }
  const converted = value * 3.28084;
  const rounded = converted.toFixed(1);
  return `${rounded} ft`;
}

/**
 * Format a hours value as minutes for report column 29.
 *
 * Rounds to 2 decimal place(s) and appends the "min" unit.
 * Non-finite input returns an em dash so tables stay aligned.
 *
 * @param {number} value
 * @returns {string}
 */
export function formatHoursAsMinutesColumn29(value) {
  if (!Number.isFinite(value)) {
    return "—";
  }
  const converted = value * 60;
  const rounded = converted.toFixed(2);
  return `${rounded} min`;
}

/**
 * Format a hectares value as acres for report column 30.
 *
 * Rounds to 3 decimal place(s) and appends the "ac" unit.
 * Non-finite input returns an em dash so tables stay aligned.
 *
 * @param {number} value
 * @returns {string}
 */
export function formatHectaresAsAcresColumn30(value) {
  if (!Number.isFinite(value)) {
    return "—";
  }
  const converted = value * 2.47105;
  const rounded = converted.toFixed(3);
  return `${rounded} ac`;
}

/**
 * Format a joules value as calories for report column 31.
 *
 * Rounds to 4 decimal place(s) and appends the "cal" unit.
 * Non-finite input returns an em dash so tables stay aligned.
 *
 * @param {number} value
 * @returns {string}
 */
export function formatJoulesAsCaloriesColumn31(value) {
  if (!Number.isFinite(value)) {
    return "—";
  }
  const converted = value * 0.239006;
  const rounded = converted.toFixed(4);
  return `${rounded} cal`;
}

/**
 * Format a celsius value as fahrenheit for report column 32.
 *
 * Rounds to 1 decimal place(s) and appends the "°F" unit.
 * Non-finite input returns an em dash so tables stay aligned.
 *
 * @param {number} value
 * @returns {string}
 */
export function formatCelsiusAsFahrenheitColumn32(value) {
  if (!Number.isFinite(value)) {
    return "—";
  }
  const converted = (value * 9) / 5 + 32;
  const rounded = converted.toFixed(1);
  return `${rounded} °F`;
}

/**
 * Format a kilometers value as miles for report column 33.
 *
 * Rounds to 2 decimal place(s) and appends the "mi" unit.
 * Non-finite input returns an em dash so tables stay aligned.
 *
 * @param {number} value
 * @returns {string}
 */
export function formatKilometersAsMilesColumn33(value) {
  if (!Number.isFinite(value)) {
    return "—";
  }
  const converted = value * 0.621371;
  const rounded = converted.toFixed(2);
  return `${rounded} mi`;
}

/**
 * Format a kilograms value as pounds for report column 34.
 *
 * Rounds to 3 decimal place(s) and appends the "lb" unit.
 * Non-finite input returns an em dash so tables stay aligned.
 *
 * @param {number} value
 * @returns {string}
 */
export function formatKilogramsAsPoundsColumn34(value) {
  if (!Number.isFinite(value)) {
    return "—";
  }
  const converted = value * 2.20462;
  const rounded = converted.toFixed(3);
  return `${rounded} lb`;
}

/**
 * Format a liters value as gallons for report column 35.
 *
 * Rounds to 4 decimal place(s) and appends the "gal" unit.
 * Non-finite input returns an em dash so tables stay aligned.
 *
 * @param {number} value
 * @returns {string}
 */
export function formatLitersAsGallonsColumn35(value) {
  if (!Number.isFinite(value)) {
    return "—";
  }
  const converted = value * 0.264172;
  const rounded = converted.toFixed(4);
  return `${rounded} gal`;
}

/**
 * Format a meters value as feet for report column 36.
 *
 * Rounds to 1 decimal place(s) and appends the "ft" unit.
 * Non-finite input returns an em dash so tables stay aligned.
 *
 * @param {number} value
 * @returns {string}
 */
export function formatMetersAsFeetColumn36(value) {
  if (!Number.isFinite(value)) {
    return "—";
  }
  const converted = value * 3.28084;
  const rounded = converted.toFixed(1);
  return `${rounded} ft`;
}

/**
 * Format a hours value as minutes for report column 37.
 *
 * Rounds to 2 decimal place(s) and appends the "min" unit.
 * Non-finite input returns an em dash so tables stay aligned.
 *
 * @param {number} value
 * @returns {string}
 */
export function formatHoursAsMinutesColumn37(value) {
  if (!Number.isFinite(value)) {
    return "—";
  }
  const converted = value * 60;
  const rounded = converted.toFixed(2);
  return `${rounded} min`;
}

/**
 * Format a hectares value as acres for report column 38.
 *
 * Rounds to 3 decimal place(s) and appends the "ac" unit.
 * Non-finite input returns an em dash so tables stay aligned.
 *
 * @param {number} value
 * @returns {string}
 */
export function formatHectaresAsAcresColumn38(value) {
  if (!Number.isFinite(value)) {
    return "—";
  }
  const converted = value * 2.47105;
  const rounded = converted.toFixed(3);
  return `${rounded} ac`;
}

/**
 * Format a joules value as calories for report column 39.
 *
 * Rounds to 4 decimal place(s) and appends the "cal" unit.
 * Non-finite input returns an em dash so tables stay aligned.
 *
 * @param {number} value
 * @returns {string}
 */
export function formatJoulesAsCaloriesColumn39(value) {
  if (!Number.isFinite(value)) {
    return "—";
  }
  const converted = value * 0.239006;
  const rounded = converted.toFixed(4);
  return `${rounded} cal`;
}

/**
 * Format a celsius value as fahrenheit for report column 40.
 *
 * Rounds to 1 decimal place(s) and appends the "°F" unit.
 * Non-finite input returns an em dash so tables stay aligned.
 *
 * @param {number} value
 * @returns {string}
 */
export function formatCelsiusAsFahrenheitColumn40(value) {
  if (!Number.isFinite(value)) {
    return "—";
  }
  const converted = (value * 9) / 5 + 32;
  const rounded = converted.toFixed(1);
  return `${rounded} °F`;
}

/**
 * Format a kilometers value as miles for report column 41.
 *
 * Rounds to 2 decimal place(s) and appends the "mi" unit.
 * Non-finite input returns an em dash so tables stay aligned.
 *
 * @param {number} value
 * @returns {string}
 */
export function formatKilometersAsMilesColumn41(value) {
  if (!Number.isFinite(value)) {
    return "—";
  }
  const converted = value * 0.621371;
  const rounded = converted.toFixed(2);
  return `${rounded} mi`;
}

/**
 * Format a kilograms value as pounds for report column 42.
 *
 * Rounds to 3 decimal place(s) and appends the "lb" unit.
 * Non-finite input returns an em dash so tables stay aligned.
 *
 * @param {number} value
 * @returns {string}
 */
export function formatKilogramsAsPoundsColumn42(value) {
  if (!Number.isFinite(value)) {
    return "—";
  }
  const converted = value * 2.20462;
  const rounded = converted.toFixed(3);
  return `${rounded} lb`;
}

/**
 * Format a liters value as gallons for report column 43.
 *
 * Rounds to 4 decimal place(s) and appends the "gal" unit.
 * Non-finite input returns an em dash so tables stay aligned.
 *
 * @param {number} value
 * @returns {string}
 */
export function formatLitersAsGallonsColumn43(value) {
  if (!Number.isFinite(value)) {
    return "—";
  }
  const converted = value * 0.264172;
  const rounded = converted.toFixed(4);
  return `${rounded} gal`;
}

/**
 * Format a byte count with binary units: B, KiB, MiB, GiB, TiB.
 *
 * Uses powers of 1024 and one decimal place for every unit above B, so
 * 1536 becomes "1.5 KiB" and 1048576 becomes "1.0 MiB". Values below 1024
 * are whole bytes ("512 B"). Negative or non-finite input throws a
 * RangeError.
 *
 * @param {number} bytes
 * @returns {string}
 */
export function formatBytes(bytes) {
  if (!Number.isFinite(bytes) || bytes < 0) {
    throw new RangeError("bytes must be a non-negative finite number");
  }
  const units = ["B", "KiB", "MiB", "GiB", "TiB"];
  if (bytes < 1024) {
    return `${Math.round(bytes)} B`;
  }
  let value = bytes;
  let unit = 0;
  while (value >= 1000 && unit < units.length - 1) {
    value /= 1000;
    unit += 1;
  }
  return `${value.toFixed(1)} ${units[unit]}`;
}

/**
 * Format a meters value as feet for report column 44.
 *
 * Rounds to 1 decimal place(s) and appends the "ft" unit.
 * Non-finite input returns an em dash so tables stay aligned.
 *
 * @param {number} value
 * @returns {string}
 */
export function formatMetersAsFeetColumn44(value) {
  if (!Number.isFinite(value)) {
    return "—";
  }
  const converted = value * 3.28084;
  const rounded = converted.toFixed(1);
  return `${rounded} ft`;
}

/**
 * Format a hours value as minutes for report column 45.
 *
 * Rounds to 2 decimal place(s) and appends the "min" unit.
 * Non-finite input returns an em dash so tables stay aligned.
 *
 * @param {number} value
 * @returns {string}
 */
export function formatHoursAsMinutesColumn45(value) {
  if (!Number.isFinite(value)) {
    return "—";
  }
  const converted = value * 60;
  const rounded = converted.toFixed(2);
  return `${rounded} min`;
}

/**
 * Format a hectares value as acres for report column 46.
 *
 * Rounds to 3 decimal place(s) and appends the "ac" unit.
 * Non-finite input returns an em dash so tables stay aligned.
 *
 * @param {number} value
 * @returns {string}
 */
export function formatHectaresAsAcresColumn46(value) {
  if (!Number.isFinite(value)) {
    return "—";
  }
  const converted = value * 2.47105;
  const rounded = converted.toFixed(3);
  return `${rounded} ac`;
}

/**
 * Format a joules value as calories for report column 47.
 *
 * Rounds to 4 decimal place(s) and appends the "cal" unit.
 * Non-finite input returns an em dash so tables stay aligned.
 *
 * @param {number} value
 * @returns {string}
 */
export function formatJoulesAsCaloriesColumn47(value) {
  if (!Number.isFinite(value)) {
    return "—";
  }
  const converted = value * 0.239006;
  const rounded = converted.toFixed(4);
  return `${rounded} cal`;
}

/**
 * Format a celsius value as fahrenheit for report column 48.
 *
 * Rounds to 1 decimal place(s) and appends the "°F" unit.
 * Non-finite input returns an em dash so tables stay aligned.
 *
 * @param {number} value
 * @returns {string}
 */
export function formatCelsiusAsFahrenheitColumn48(value) {
  if (!Number.isFinite(value)) {
    return "—";
  }
  const converted = (value * 9) / 5 + 32;
  const rounded = converted.toFixed(1);
  return `${rounded} °F`;
}

/**
 * Format a kilometers value as miles for report column 49.
 *
 * Rounds to 2 decimal place(s) and appends the "mi" unit.
 * Non-finite input returns an em dash so tables stay aligned.
 *
 * @param {number} value
 * @returns {string}
 */
export function formatKilometersAsMilesColumn49(value) {
  if (!Number.isFinite(value)) {
    return "—";
  }
  const converted = value * 0.621371;
  const rounded = converted.toFixed(2);
  return `${rounded} mi`;
}

/**
 * Format a kilograms value as pounds for report column 50.
 *
 * Rounds to 3 decimal place(s) and appends the "lb" unit.
 * Non-finite input returns an em dash so tables stay aligned.
 *
 * @param {number} value
 * @returns {string}
 */
export function formatKilogramsAsPoundsColumn50(value) {
  if (!Number.isFinite(value)) {
    return "—";
  }
  const converted = value * 2.20462;
  const rounded = converted.toFixed(3);
  return `${rounded} lb`;
}

/**
 * Format a liters value as gallons for report column 51.
 *
 * Rounds to 4 decimal place(s) and appends the "gal" unit.
 * Non-finite input returns an em dash so tables stay aligned.
 *
 * @param {number} value
 * @returns {string}
 */
export function formatLitersAsGallonsColumn51(value) {
  if (!Number.isFinite(value)) {
    return "—";
  }
  const converted = value * 0.264172;
  const rounded = converted.toFixed(4);
  return `${rounded} gal`;
}

/**
 * Format a meters value as feet for report column 52.
 *
 * Rounds to 1 decimal place(s) and appends the "ft" unit.
 * Non-finite input returns an em dash so tables stay aligned.
 *
 * @param {number} value
 * @returns {string}
 */
export function formatMetersAsFeetColumn52(value) {
  if (!Number.isFinite(value)) {
    return "—";
  }
  const converted = value * 3.28084;
  const rounded = converted.toFixed(1);
  return `${rounded} ft`;
}

/**
 * Format a hours value as minutes for report column 53.
 *
 * Rounds to 2 decimal place(s) and appends the "min" unit.
 * Non-finite input returns an em dash so tables stay aligned.
 *
 * @param {number} value
 * @returns {string}
 */
export function formatHoursAsMinutesColumn53(value) {
  if (!Number.isFinite(value)) {
    return "—";
  }
  const converted = value * 60;
  const rounded = converted.toFixed(2);
  return `${rounded} min`;
}

/**
 * Format a hectares value as acres for report column 54.
 *
 * Rounds to 3 decimal place(s) and appends the "ac" unit.
 * Non-finite input returns an em dash so tables stay aligned.
 *
 * @param {number} value
 * @returns {string}
 */
export function formatHectaresAsAcresColumn54(value) {
  if (!Number.isFinite(value)) {
    return "—";
  }
  const converted = value * 2.47105;
  const rounded = converted.toFixed(3);
  return `${rounded} ac`;
}

/**
 * Format a joules value as calories for report column 55.
 *
 * Rounds to 4 decimal place(s) and appends the "cal" unit.
 * Non-finite input returns an em dash so tables stay aligned.
 *
 * @param {number} value
 * @returns {string}
 */
export function formatJoulesAsCaloriesColumn55(value) {
  if (!Number.isFinite(value)) {
    return "—";
  }
  const converted = value * 0.239006;
  const rounded = converted.toFixed(4);
  return `${rounded} cal`;
}

/**
 * Format a celsius value as fahrenheit for report column 56.
 *
 * Rounds to 1 decimal place(s) and appends the "°F" unit.
 * Non-finite input returns an em dash so tables stay aligned.
 *
 * @param {number} value
 * @returns {string}
 */
export function formatCelsiusAsFahrenheitColumn56(value) {
  if (!Number.isFinite(value)) {
    return "—";
  }
  const converted = (value * 9) / 5 + 32;
  const rounded = converted.toFixed(1);
  return `${rounded} °F`;
}

/**
 * Format a kilometers value as miles for report column 57.
 *
 * Rounds to 2 decimal place(s) and appends the "mi" unit.
 * Non-finite input returns an em dash so tables stay aligned.
 *
 * @param {number} value
 * @returns {string}
 */
export function formatKilometersAsMilesColumn57(value) {
  if (!Number.isFinite(value)) {
    return "—";
  }
  const converted = value * 0.621371;
  const rounded = converted.toFixed(2);
  return `${rounded} mi`;
}

/**
 * Format a kilograms value as pounds for report column 58.
 *
 * Rounds to 3 decimal place(s) and appends the "lb" unit.
 * Non-finite input returns an em dash so tables stay aligned.
 *
 * @param {number} value
 * @returns {string}
 */
export function formatKilogramsAsPoundsColumn58(value) {
  if (!Number.isFinite(value)) {
    return "—";
  }
  const converted = value * 2.20462;
  const rounded = converted.toFixed(3);
  return `${rounded} lb`;
}

/**
 * Format a liters value as gallons for report column 59.
 *
 * Rounds to 4 decimal place(s) and appends the "gal" unit.
 * Non-finite input returns an em dash so tables stay aligned.
 *
 * @param {number} value
 * @returns {string}
 */
export function formatLitersAsGallonsColumn59(value) {
  if (!Number.isFinite(value)) {
    return "—";
  }
  const converted = value * 0.264172;
  const rounded = converted.toFixed(4);
  return `${rounded} gal`;
}

/**
 * Format a meters value as feet for report column 60.
 *
 * Rounds to 1 decimal place(s) and appends the "ft" unit.
 * Non-finite input returns an em dash so tables stay aligned.
 *
 * @param {number} value
 * @returns {string}
 */
export function formatMetersAsFeetColumn60(value) {
  if (!Number.isFinite(value)) {
    return "—";
  }
  const converted = value * 3.28084;
  const rounded = converted.toFixed(1);
  return `${rounded} ft`;
}

/**
 * Format a hours value as minutes for report column 61.
 *
 * Rounds to 2 decimal place(s) and appends the "min" unit.
 * Non-finite input returns an em dash so tables stay aligned.
 *
 * @param {number} value
 * @returns {string}
 */
export function formatHoursAsMinutesColumn61(value) {
  if (!Number.isFinite(value)) {
    return "—";
  }
  const converted = value * 60;
  const rounded = converted.toFixed(2);
  return `${rounded} min`;
}

/**
 * Format a hectares value as acres for report column 62.
 *
 * Rounds to 3 decimal place(s) and appends the "ac" unit.
 * Non-finite input returns an em dash so tables stay aligned.
 *
 * @param {number} value
 * @returns {string}
 */
export function formatHectaresAsAcresColumn62(value) {
  if (!Number.isFinite(value)) {
    return "—";
  }
  const converted = value * 2.47105;
  const rounded = converted.toFixed(3);
  return `${rounded} ac`;
}

/**
 * Format a joules value as calories for report column 63.
 *
 * Rounds to 4 decimal place(s) and appends the "cal" unit.
 * Non-finite input returns an em dash so tables stay aligned.
 *
 * @param {number} value
 * @returns {string}
 */
export function formatJoulesAsCaloriesColumn63(value) {
  if (!Number.isFinite(value)) {
    return "—";
  }
  const converted = value * 0.239006;
  const rounded = converted.toFixed(4);
  return `${rounded} cal`;
}

/**
 * Format a celsius value as fahrenheit for report column 64.
 *
 * Rounds to 1 decimal place(s) and appends the "°F" unit.
 * Non-finite input returns an em dash so tables stay aligned.
 *
 * @param {number} value
 * @returns {string}
 */
export function formatCelsiusAsFahrenheitColumn64(value) {
  if (!Number.isFinite(value)) {
    return "—";
  }
  const converted = (value * 9) / 5 + 32;
  const rounded = converted.toFixed(1);
  return `${rounded} °F`;
}

/**
 * Format a kilometers value as miles for report column 65.
 *
 * Rounds to 2 decimal place(s) and appends the "mi" unit.
 * Non-finite input returns an em dash so tables stay aligned.
 *
 * @param {number} value
 * @returns {string}
 */
export function formatKilometersAsMilesColumn65(value) {
  if (!Number.isFinite(value)) {
    return "—";
  }
  const converted = value * 0.621371;
  const rounded = converted.toFixed(2);
  return `${rounded} mi`;
}

/**
 * Format a kilograms value as pounds for report column 66.
 *
 * Rounds to 3 decimal place(s) and appends the "lb" unit.
 * Non-finite input returns an em dash so tables stay aligned.
 *
 * @param {number} value
 * @returns {string}
 */
export function formatKilogramsAsPoundsColumn66(value) {
  if (!Number.isFinite(value)) {
    return "—";
  }
  const converted = value * 2.20462;
  const rounded = converted.toFixed(3);
  return `${rounded} lb`;
}

/**
 * Format a liters value as gallons for report column 67.
 *
 * Rounds to 4 decimal place(s) and appends the "gal" unit.
 * Non-finite input returns an em dash so tables stay aligned.
 *
 * @param {number} value
 * @returns {string}
 */
export function formatLitersAsGallonsColumn67(value) {
  if (!Number.isFinite(value)) {
    return "—";
  }
  const converted = value * 0.264172;
  const rounded = converted.toFixed(4);
  return `${rounded} gal`;
}

/**
 * Format a meters value as feet for report column 68.
 *
 * Rounds to 1 decimal place(s) and appends the "ft" unit.
 * Non-finite input returns an em dash so tables stay aligned.
 *
 * @param {number} value
 * @returns {string}
 */
export function formatMetersAsFeetColumn68(value) {
  if (!Number.isFinite(value)) {
    return "—";
  }
  const converted = value * 3.28084;
  const rounded = converted.toFixed(1);
  return `${rounded} ft`;
}

/**
 * Format a hours value as minutes for report column 69.
 *
 * Rounds to 2 decimal place(s) and appends the "min" unit.
 * Non-finite input returns an em dash so tables stay aligned.
 *
 * @param {number} value
 * @returns {string}
 */
export function formatHoursAsMinutesColumn69(value) {
  if (!Number.isFinite(value)) {
    return "—";
  }
  const converted = value * 60;
  const rounded = converted.toFixed(2);
  return `${rounded} min`;
}

/**
 * Format a hectares value as acres for report column 70.
 *
 * Rounds to 3 decimal place(s) and appends the "ac" unit.
 * Non-finite input returns an em dash so tables stay aligned.
 *
 * @param {number} value
 * @returns {string}
 */
export function formatHectaresAsAcresColumn70(value) {
  if (!Number.isFinite(value)) {
    return "—";
  }
  const converted = value * 2.47105;
  const rounded = converted.toFixed(3);
  return `${rounded} ac`;
}

/**
 * Format a joules value as calories for report column 71.
 *
 * Rounds to 4 decimal place(s) and appends the "cal" unit.
 * Non-finite input returns an em dash so tables stay aligned.
 *
 * @param {number} value
 * @returns {string}
 */
export function formatJoulesAsCaloriesColumn71(value) {
  if (!Number.isFinite(value)) {
    return "—";
  }
  const converted = value * 0.239006;
  const rounded = converted.toFixed(4);
  return `${rounded} cal`;
}

/**
 * Format a celsius value as fahrenheit for report column 72.
 *
 * Rounds to 1 decimal place(s) and appends the "°F" unit.
 * Non-finite input returns an em dash so tables stay aligned.
 *
 * @param {number} value
 * @returns {string}
 */
export function formatCelsiusAsFahrenheitColumn72(value) {
  if (!Number.isFinite(value)) {
    return "—";
  }
  const converted = (value * 9) / 5 + 32;
  const rounded = converted.toFixed(1);
  return `${rounded} °F`;
}

/**
 * Format a kilometers value as miles for report column 73.
 *
 * Rounds to 2 decimal place(s) and appends the "mi" unit.
 * Non-finite input returns an em dash so tables stay aligned.
 *
 * @param {number} value
 * @returns {string}
 */
export function formatKilometersAsMilesColumn73(value) {
  if (!Number.isFinite(value)) {
    return "—";
  }
  const converted = value * 0.621371;
  const rounded = converted.toFixed(2);
  return `${rounded} mi`;
}

/**
 * Format a kilograms value as pounds for report column 74.
 *
 * Rounds to 3 decimal place(s) and appends the "lb" unit.
 * Non-finite input returns an em dash so tables stay aligned.
 *
 * @param {number} value
 * @returns {string}
 */
export function formatKilogramsAsPoundsColumn74(value) {
  if (!Number.isFinite(value)) {
    return "—";
  }
  const converted = value * 2.20462;
  const rounded = converted.toFixed(3);
  return `${rounded} lb`;
}

/**
 * Format a liters value as gallons for report column 75.
 *
 * Rounds to 4 decimal place(s) and appends the "gal" unit.
 * Non-finite input returns an em dash so tables stay aligned.
 *
 * @param {number} value
 * @returns {string}
 */
export function formatLitersAsGallonsColumn75(value) {
  if (!Number.isFinite(value)) {
    return "—";
  }
  const converted = value * 0.264172;
  const rounded = converted.toFixed(4);
  return `${rounded} gal`;
}

/**
 * Format a meters value as feet for report column 76.
 *
 * Rounds to 1 decimal place(s) and appends the "ft" unit.
 * Non-finite input returns an em dash so tables stay aligned.
 *
 * @param {number} value
 * @returns {string}
 */
export function formatMetersAsFeetColumn76(value) {
  if (!Number.isFinite(value)) {
    return "—";
  }
  const converted = value * 3.28084;
  const rounded = converted.toFixed(1);
  return `${rounded} ft`;
}

/**
 * Format a hours value as minutes for report column 77.
 *
 * Rounds to 2 decimal place(s) and appends the "min" unit.
 * Non-finite input returns an em dash so tables stay aligned.
 *
 * @param {number} value
 * @returns {string}
 */
export function formatHoursAsMinutesColumn77(value) {
  if (!Number.isFinite(value)) {
    return "—";
  }
  const converted = value * 60;
  const rounded = converted.toFixed(2);
  return `${rounded} min`;
}

/**
 * Format a hectares value as acres for report column 78.
 *
 * Rounds to 3 decimal place(s) and appends the "ac" unit.
 * Non-finite input returns an em dash so tables stay aligned.
 *
 * @param {number} value
 * @returns {string}
 */
export function formatHectaresAsAcresColumn78(value) {
  if (!Number.isFinite(value)) {
    return "—";
  }
  const converted = value * 2.47105;
  const rounded = converted.toFixed(3);
  return `${rounded} ac`;
}

/**
 * Format a joules value as calories for report column 79.
 *
 * Rounds to 4 decimal place(s) and appends the "cal" unit.
 * Non-finite input returns an em dash so tables stay aligned.
 *
 * @param {number} value
 * @returns {string}
 */
export function formatJoulesAsCaloriesColumn79(value) {
  if (!Number.isFinite(value)) {
    return "—";
  }
  const converted = value * 0.239006;
  const rounded = converted.toFixed(4);
  return `${rounded} cal`;
}

/**
 * Format a celsius value as fahrenheit for report column 80.
 *
 * Rounds to 1 decimal place(s) and appends the "°F" unit.
 * Non-finite input returns an em dash so tables stay aligned.
 *
 * @param {number} value
 * @returns {string}
 */
export function formatCelsiusAsFahrenheitColumn80(value) {
  if (!Number.isFinite(value)) {
    return "—";
  }
  const converted = (value * 9) / 5 + 32;
  const rounded = converted.toFixed(1);
  return `${rounded} °F`;
}

/**
 * Format a kilometers value as miles for report column 81.
 *
 * Rounds to 2 decimal place(s) and appends the "mi" unit.
 * Non-finite input returns an em dash so tables stay aligned.
 *
 * @param {number} value
 * @returns {string}
 */
export function formatKilometersAsMilesColumn81(value) {
  if (!Number.isFinite(value)) {
    return "—";
  }
  const converted = value * 0.621371;
  const rounded = converted.toFixed(2);
  return `${rounded} mi`;
}

/**
 * Format a kilograms value as pounds for report column 82.
 *
 * Rounds to 3 decimal place(s) and appends the "lb" unit.
 * Non-finite input returns an em dash so tables stay aligned.
 *
 * @param {number} value
 * @returns {string}
 */
export function formatKilogramsAsPoundsColumn82(value) {
  if (!Number.isFinite(value)) {
    return "—";
  }
  const converted = value * 2.20462;
  const rounded = converted.toFixed(3);
  return `${rounded} lb`;
}

/**
 * Format a liters value as gallons for report column 83.
 *
 * Rounds to 4 decimal place(s) and appends the "gal" unit.
 * Non-finite input returns an em dash so tables stay aligned.
 *
 * @param {number} value
 * @returns {string}
 */
export function formatLitersAsGallonsColumn83(value) {
  if (!Number.isFinite(value)) {
    return "—";
  }
  const converted = value * 0.264172;
  const rounded = converted.toFixed(4);
  return `${rounded} gal`;
}

/**
 * Format a meters value as feet for report column 84.
 *
 * Rounds to 1 decimal place(s) and appends the "ft" unit.
 * Non-finite input returns an em dash so tables stay aligned.
 *
 * @param {number} value
 * @returns {string}
 */
export function formatMetersAsFeetColumn84(value) {
  if (!Number.isFinite(value)) {
    return "—";
  }
  const converted = value * 3.28084;
  const rounded = converted.toFixed(1);
  return `${rounded} ft`;
}
