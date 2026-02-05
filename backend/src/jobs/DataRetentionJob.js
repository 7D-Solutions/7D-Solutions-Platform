const { billingPrisma } = require('../prisma');
const logger = require('@fireproof/infrastructure/utils/logger');

/**
 * DataRetentionJob - Scheduled job for purging/archiving old data
 *
 * Configuration:
 * - retentionDays: Default retention period for each table (days)
 * - tables: List of tables to process with safe defaults
 *
 * IMPORTANT: Financial records must be retained for 7+ years for tax compliance.
 * This job only purges non‑financial, operational data (webhook attempts, idempotency keys, events).
 */
class DataRetentionJob {
  constructor() {
    // Retention periods in days (defaults)
    this.retentionConfig = {
      billing_webhook_attempts: 90,      // 3 months
      billing_idempotency_keys: 30,      // 1 month
      billing_events: 365,               // 1 year
      // Add other non‑financial tables as needed
    };

    // Batch size for deletion queries
    this.batchSize = 1000;
  }

  /**
   * Calculate cutoff date based on retention days
   * @param {number} retentionDays
   * @returns {Date}
   */
  getCutoffDate(retentionDays) {
    const cutoff = new Date();
    cutoff.setDate(cutoff.getDate() - retentionDays);
    return cutoff;
  }

  /**
   * Purge old records from a single table
   * @param {string} tableName - Prisma model name
   * @param {number} retentionDays - Days to keep
   * @returns {Promise<number>} Number of records deleted
   */
  async purgeTable(tableName, retentionDays) {
    const cutoff = this.getCutoffDate(retentionDays);
    let totalDeleted = 0;

    try {
      // Use batch deletion to avoid locking
      let batchDeleted;
      do {
        // Use Prisma's deleteMany with limit workaround (fetch IDs first)
        // Since Prisma doesn't support LIMIT in deleteMany, we use a transaction
        const recordsToDelete = await billingPrisma[tableName].findMany({
          where: { created_at: { lt: cutoff } },
          select: { id: true },
          take: this.batchSize,
          orderBy: { created_at: 'asc' }
        });

        if (recordsToDelete.length === 0) {
          break;
        }

        const ids = recordsToDelete.map(r => r.id);
        batchDeleted = await billingPrisma[tableName].deleteMany({
          where: { id: { in: ids } }
        });

        totalDeleted += batchDeleted.count;
        logger.debug(`Purged batch of ${batchDeleted.count} records from ${tableName}`, {
          table: tableName,
          batch: batchDeleted.count,
          total: totalDeleted
        });

        // Small delay to reduce DB load
        await new Promise(resolve => setTimeout(resolve, 100));
      } while (batchDeleted.count === this.batchSize);

      logger.info(`Purged ${totalDeleted} records from ${tableName} (older than ${retentionDays} days)`, {
        table: tableName,
        retentionDays,
        totalDeleted
      });

      return totalDeleted;
    } catch (error) {
      logger.error(`Failed to purge table ${tableName}`, {
        table: tableName,
        error: error.message,
        stack: error.stack
      });
      throw error;
    }
  }

  /**
   * Archive old records (placeholder for future implementation)
   * @param {string} tableName
   * @param {number} retentionDays
   */
  async archiveTable(tableName, retentionDays) {
    logger.info('Archive placeholder', { tableName, retentionDays });
    // TODO: Implement archive logic (copy to separate table, then delete)
    // For now, just log that archiving would be needed for financial data
    return 0;
  }

  /**
   * Determine whether to purge or archive based on table type
   * @param {string} tableName
   * @returns {string} 'purge' or 'archive'
   */
  getActionForTable(tableName) {
    // Non‑financial operational tables can be purged
    const purgeTables = ['billing_webhook_attempts', 'billing_idempotency_keys', 'billing_events'];
    if (purgeTables.includes(tableName)) {
      return 'purge';
    }
    // Financial tables must be archived, never purged
    return 'archive';
  }

  /**
   * Process a single table according to its retention config
   * @param {string} tableName
   * @returns {Promise<Object>} Result
   */
  async processTable(tableName) {
    const retentionDays = this.retentionConfig[tableName];
    if (!retentionDays) {
      logger.warn(`No retention configuration for table ${tableName}, skipping`);
      return { tableName, action: 'skip', reason: 'no_config' };
    }

    const action = this.getActionForTable(tableName);
    let processedCount = 0;

    try {
      if (action === 'purge') {
        processedCount = await this.purgeTable(tableName, retentionDays);
      } else {
        processedCount = await this.archiveTable(tableName, retentionDays);
      }
      return {
        tableName,
        action,
        retentionDays,
        processedCount,
        success: true
      };
    } catch (error) {
      return {
        tableName,
        action,
        retentionDays,
        processedCount,
        success: false,
        error: error.message
      };
    }
  }

  /**
   * Main job entry point: process all configured tables
   * @param {Object} options
   * @param {string} options.appId - Filter by app (not yet supported)
   * @returns {Promise<Object>} Summary of processed tables
   */
  async runDataRetentionJob(options = {}) {
    const { appId } = options;
    const startTime = Date.now();
    logger.info('Starting data retention job', { app_id: appId });

    const tables = Object.keys(this.retentionConfig);
    const results = {
      processedTables: 0,
      purged: 0,
      archived: 0,
      skipped: 0,
      errors: 0,
      details: []
    };

    for (const tableName of tables) {
      const tableResult = await this.processTable(tableName);
      results.details.push(tableResult);
      results.processedTables++;
      if (tableResult.success) {
        if (tableResult.action === 'purge') results.purged++;
        if (tableResult.action === 'archive') results.archived++;
      } else {
        results.errors++;
        results.skipped++;
      }
    }

    const duration = Date.now() - startTime;
    logger.info('Data retention job completed', {
      app_id: appId,
      duration: `${duration}ms`,
      ...results
    });

    return results;
  }
}

module.exports = DataRetentionJob;