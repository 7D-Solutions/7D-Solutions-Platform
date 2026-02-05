const TilledClient = require('../../backend/src/tilledClient');
const { generateWebhookSignature } = require('../fixtures/test-fixtures');

describe('TilledClient', () => {
  let client;
  let mockTilledSDK;

  beforeEach(() => {
    process.env.TILLED_SECRET_KEY_TRASHTECH = 'sk_test_123';
    process.env.TILLED_ACCOUNT_ID_TRASHTECH = 'acct_123';
    process.env.TILLED_WEBHOOK_SECRET_TRASHTECH = 'whsec_123';
    process.env.TILLED_SANDBOX = 'true';

    // Mock tilled-node SDK
    mockTilledSDK = {
      Configuration: jest.fn(),
      CustomersApi: jest.fn(),
      SubscriptionsApi: jest.fn(),
      PaymentMethodsApi: jest.fn()
    };

    jest.mock('tilled-node', () => mockTilledSDK);

    client = new TilledClient('trashtech');
  });

  afterEach(() => {
    jest.clearAllMocks();
  });

  describe('loadConfig', () => {
    it('should load config from environment variables', () => {
      const config = client.config;

      expect(config.secretKey).toBe('sk_test_123');
      expect(config.accountId).toBe('acct_123');
      expect(config.webhookSecret).toBe('whsec_123');
      expect(config.sandbox).toBe(true);
      expect(config.basePath).toBe('https://sandbox-api.tilled.com');
    });

    it('should use production URL when sandbox=false', () => {
      process.env.TILLED_SANDBOX = 'false';
      const prodClient = new TilledClient('trashtech');

      expect(prodClient.config.basePath).toBe('https://api.tilled.com');
    });

    it('should throw error if credentials missing', () => {
      delete process.env.TILLED_SECRET_KEY_TRASHTECH;

      expect(() => new TilledClient('trashtech')).toThrow('Missing Tilled config for app: trashtech');
    });
  });

  describe('verifyWebhookSignature', () => {
    const rawBody = JSON.stringify({ id: 'evt_test', type: 'test' });
    const secret = 'whsec_123';

    it('should verify valid signature', () => {
      const signature = generateWebhookSignature(JSON.parse(rawBody), secret);

      const result = client.verifyWebhookSignature(rawBody, signature);

      expect(result).toBe(true);
    });

    it('should reject invalid signature', () => {
      const signature = 't=1234567890,v1=invalidsignature';

      const result = client.verifyWebhookSignature(rawBody, signature);

      expect(result).toBe(false);
    });

    it('should reject signature with timestamp outside tolerance', () => {
      const oldTimestamp = (Math.floor(Date.now() / 1000) - 10 * 60).toString(); // 10 minutes ago
      const signature = generateWebhookSignature(JSON.parse(rawBody), secret, oldTimestamp);

      const result = client.verifyWebhookSignature(rawBody, signature, 300); // 5 min tolerance

      expect(result).toBe(false);
    });

    it('should accept signature within tolerance', () => {
      const recentTimestamp = Math.floor(Date.now() / 1000).toString();
      const signature = generateWebhookSignature(JSON.parse(rawBody), secret, recentTimestamp);

      const result = client.verifyWebhookSignature(rawBody, signature, 300);

      expect(result).toBe(true);
    });

    it('should reject signature with mismatched length', () => {
      const signature = 't=1234567890,v1=short';

      const result = client.verifyWebhookSignature(rawBody, signature);

      expect(result).toBe(false);
    });

    it('should reject missing signature', () => {
      const result = client.verifyWebhookSignature(rawBody, null);

      expect(result).toBe(false);
    });

    it('should reject missing raw body', () => {
      const signature = generateWebhookSignature(JSON.parse(rawBody), secret);

      const result = client.verifyWebhookSignature(null, signature);

      expect(result).toBe(false);
    });

    it('should reject malformed signature format', () => {
      const signature = 'invalid-format';

      const result = client.verifyWebhookSignature(rawBody, signature);

      expect(result).toBe(false);
    });
  });

  describe('SDK initialization', () => {
    it('should initialize SDK only once', () => {
      mockTilledSDK.Configuration.mockImplementation(() => ({}));
      mockTilledSDK.CustomersApi.mockImplementation(() => ({}));
      mockTilledSDK.SubscriptionsApi.mockImplementation(() => ({}));
      mockTilledSDK.PaymentMethodsApi.mockImplementation(() => ({}));

      client.initializeSDK();
      client.initializeSDK();
      client.initializeSDK();

      // Should only call constructors once
      expect(mockTilledSDK.Configuration).toHaveBeenCalledTimes(1);
    });

    it('should initialize with correct configuration', () => {
      mockTilledSDK.Configuration.mockImplementation((config) => {
        expect(config.apiKey).toBe('sk_test_123');
        expect(config.basePath).toBe('https://sandbox-api.tilled.com');
        return {};
      });

      client.initializeSDK();
    });
  });
});
