/**
 * ProrationCalculator - Pure math helpers for proration calculations
 *
 * Stateless utility functions extracted from ProrationService.
 * All methods are static and require no constructor dependencies.
 */
class ProrationCalculator {
  /**
   * Calculate days remaining in billing period
   * @param {Date} changeDate
   * @param {Date} periodEnd
   * @param {Date} periodStart
   * @returns {Object} { daysUsed, daysRemaining, daysTotal, prorationFactor }
   */
  static calculateTimeProration(changeDate, periodEnd, periodStart) {
    // Normalize all dates to midnight UTC for consistency
    const change = new Date(changeDate);
    change.setUTCHours(0, 0, 0, 0);

    const start = new Date(periodStart);
    start.setUTCHours(0, 0, 0, 0);

    const end = new Date(periodEnd);
    end.setUTCHours(0, 0, 0, 0);

    // Edge case: change at period start
    if (change.getTime() === start.getTime()) {
      return {
        daysUsed: 0,
        daysRemaining: Math.round((end - start) / (24 * 60 * 60 * 1000)),
        daysTotal: Math.round((end - start) / (24 * 60 * 60 * 1000)),
        prorationFactor: 1.0,
        note: 'change_at_period_start'
      };
    }

    // Edge case: change at or after period end
    if (change.getTime() >= end.getTime()) {
      return {
        daysUsed: Math.round((end - start) / (24 * 60 * 60 * 1000)),
        daysRemaining: 0,
        daysTotal: Math.round((end - start) / (24 * 60 * 60 * 1000)),
        prorationFactor: 0.0,
        note: 'change_at_period_end'
      };
    }

    // Calculate milliseconds
    const totalMs = end - start;
    const usedMs = change - start;
    const remainingMs = end - change;

    // Convert to days
    const msPerDay = 24 * 60 * 60 * 1000;
    const daysTotal = totalMs / msPerDay;
    const daysUsed = usedMs / msPerDay;
    const daysRemaining = remainingMs / msPerDay;

    // Calculate proration factor (4 decimal places)
    const prorationFactor = daysRemaining / daysTotal;

    return {
      daysUsed: Math.floor(daysUsed),
      daysRemaining: Math.ceil(daysRemaining),
      daysTotal: Math.round(daysTotal),
      prorationFactor: Math.round(prorationFactor * 10000) / 10000
    };
  }

  /**
   * Apply financial rounding to proration amounts
   * @param {number} amountCents
   * @returns {number} Rounded amount
   */
  static roundToFinancialStandard(amountCents) {
    // Round half-up to nearest cent
    return Math.round(amountCents);
  }

  /**
   * Normalize date to UTC midnight
   * @param {Date} date
   * @returns {Date} Normalized date
   */
  static normalizeToUTCMidnight(date) {
    const d = new Date(date);
    d.setUTCHours(0, 0, 0, 0);
    return d;
  }
}

module.exports = ProrationCalculator;
