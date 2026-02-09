const { billingPrisma } = require('../prisma');
const logger = require('@fireproof/infrastructure/utils/logger');
const { NotFoundError, ValidationError } = require('../utils/errors');

/**
 * DunningConfigService - Manages dunning configuration per app
 */
class DunningConfigService {
  /**
   * Get dunning configuration for an app.
   * Falls back to global config (app_id = null) if no app-specific config exists.
   * @param {string} appId - Application identifier
   * @returns {Promise<Object>} Dunning config object
   */
  async getConfig(appId) {
    if (!appId) {
      throw new ValidationError('appId is required');
    }

    // Try to find app-specific config first
    let config = await billingPrisma.billing_dunning_config.findFirst({
      where: { app_id: appId }
    });

    // If not found, fall back to global config (app_id = null)
    if (!config) {
      config = await billingPrisma.billing_dunning_config.findFirst({
        where: { app_id: null }
      });
    }

    // If still no config, create default global config (should be seeded)
    if (!config) {
      logger.warn('No dunning config found, creating default global config');
      config = await this.createDefaultGlobalConfig();
    }

    // Parse retry_schedule_days JSON
    let retryScheduleDays = [];
    if (config.retry_schedule_days) {
      try {
        retryScheduleDays = typeof config.retry_schedule_days === 'string'
          ? JSON.parse(config.retry_schedule_days)
          : config.retry_schedule_days;
        // Ensure it's an array
        if (!Array.isArray(retryScheduleDays)) {
          logger.warn('retry_schedule_days is not an array, defaulting to empty', {
            app_id: appId,
            retry_schedule_days: config.retry_schedule_days
          });
          retryScheduleDays = [];
        }
      } catch (error) {
        logger.error('Failed to parse retry_schedule_days JSON', {
          app_id: appId,
          error: error.message
        });
        retryScheduleDays = [];
      }
    }

    return {
      id: config.id,
      appId: config.app_id,
      gracePeriodDays: config.grace_period_days,
      retryScheduleDays,
      maxRetryAttempts: config.max_retry_attempts,
      createdAt: config.created_at,
      updatedAt: config.updated_at
    };
  }

  /**
   * Create default global dunning configuration.
   * @returns {Promise<Object>} Created config record
   */
  async createDefaultGlobalConfig() {
    const defaultConfig = {
      app_id: null,
      grace_period_days: 3,
      retry_schedule_days: JSON.stringify([1, 3, 7]),
      max_retry_attempts: 3
    };

    const config = await billingPrisma.billing_dunning_config.create({
      data: defaultConfig
    });

    logger.info('Default global dunning config created', {
      config_id: config.id
    });

    return config;
  }

  /**
   * Set dunning configuration for an app.
   * If app_id is null, updates global config.
   * @param {string} appId - Application identifier (null for global)
   * @param {Object} data - Configuration data
   * @param {number} data.gracePeriodDays - Grace period in days
   * @param {Array<number>} data.retryScheduleDays - Array of days for retry schedule
   * @param {number} data.maxRetryAttempts - Maximum retry attempts
   * @returns {Promise<Object>} Updated or created config record
   */
  async setConfig(appId, data) {
    const { gracePeriodDays, retryScheduleDays, maxRetryAttempts } = data;

    // Validate inputs
    if (gracePeriodDays !== undefined && (typeof gracePeriodDays !== 'number' || gracePeriodDays < 0)) {
      throw new ValidationError('gracePeriodDays must be a non-negative number');
    }
    if (retryScheduleDays !== undefined) {
      if (!Array.isArray(retryScheduleDays)) {
        throw new ValidationError('retryScheduleDays must be an array');
      }
      if (retryScheduleDays.some(day => typeof day !== 'number' || day < 0)) {
        throw new ValidationError('retryScheduleDays must contain non-negative numbers');
      }
    }
    if (maxRetryAttempts !== undefined && (typeof maxRetryAttempts !== 'number' || maxRetryAttempts < 0)) {
      throw new ValidationError('maxRetryAttempts must be a non-negative number');
    }

    // Prepare update data
    const updateData = {};
    if (gracePeriodDays !== undefined) updateData.grace_period_days = gracePeriodDays;
    if (retryScheduleDays !== undefined) updateData.retry_schedule_days = JSON.stringify(retryScheduleDays);
    if (maxRetryAttempts !== undefined) updateData.max_retry_attempts = maxRetryAttempts;

    // Upsert configuration
    const config = await billingPrisma.billing_dunning_config.upsert({
      where: {
        app_id: appId !== undefined ? appId : null
      },
      update: updateData,
      create: {
        app_id: appId !== undefined ? appId : null,
        grace_period_days: gracePeriodDays !== undefined ? gracePeriodDays : 3,
        retry_schedule_days: retryScheduleDays !== undefined ? JSON.stringify(retryScheduleDays) : JSON.stringify([1, 3, 7]),
        max_retry_attempts: maxRetryAttempts !== undefined ? maxRetryAttempts : 3
      }
    });

    logger.info('Dunning config updated', {
      app_id: appId,
      config_id: config.id
    });

    return config;
  }
}

module.exports = DunningConfigService;