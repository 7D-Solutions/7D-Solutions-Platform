const { captureRawBody, requireAppId, rejectSensitiveData } = require('../../backend/src/middleware');

describe('Middleware', () => {
  let req, res, next;

  beforeEach(() => {
    req = {
      setEncoding: jest.fn(),
      on: jest.fn()
    };
    res = {
      status: jest.fn().mockReturnThis(),
      json: jest.fn().mockReturnThis()
    };
    next = jest.fn();
  });

  describe('captureRawBody', () => {
    it('should capture raw body from request stream', (done) => {
      const chunks = ['{"id":', '"evt_123"}'];
      let dataHandler;
      let endHandler;

      req.on.mockImplementation((event, handler) => {
        if (event === 'data') dataHandler = handler;
        if (event === 'end') endHandler = handler;
      });

      captureRawBody(req, res, () => {
        expect(req.rawBody).toBe('{"id":"evt_123"}');
        expect(req.setEncoding).toHaveBeenCalledWith('utf8');
        done();
      });

      // Simulate streaming
      chunks.forEach(chunk => dataHandler(chunk));
      endHandler();
    });

    it('should set encoding to utf8', () => {
      captureRawBody(req, res, next);
      expect(req.setEncoding).toHaveBeenCalledWith('utf8');
    });

    it('should initialize rawBody as empty string', () => {
      captureRawBody(req, res, next);
      expect(req.rawBody).toBe('');
    });
  });

  describe('requireAppId', () => {
    it('should return 400 when app_id is missing', () => {
      req.params = {};
      req.body = {};
      req.query = {};

      const middleware = requireAppId();
      middleware(req, res, next);

      expect(res.status).toHaveBeenCalledWith(400);
      expect(res.json).toHaveBeenCalledWith({ error: 'Missing app_id' });
      expect(next).not.toHaveBeenCalled();
    });

    it('should allow request when no auth function provided', () => {
      req.params = { app_id: 'trashtech' };
      req.body = {};

      const middleware = requireAppId();
      middleware(req, res, next);

      expect(req.verifiedAppId).toBe('trashtech');
      expect(next).toHaveBeenCalledWith();
      expect(res.status).not.toHaveBeenCalled();
    });

    it('should get app_id from params', () => {
      req.params = { app_id: 'trashtech' };
      req.body = {};

      const middleware = requireAppId();
      middleware(req, res, next);

      expect(req.verifiedAppId).toBe('trashtech');
    });

    it('should get app_id from body if not in params', () => {
      req.params = {};
      req.body = { app_id: 'apping' };

      const middleware = requireAppId();
      middleware(req, res, next);

      expect(req.verifiedAppId).toBe('apping');
    });

    it('should allow request when auth function returns matching app_id', () => {
      req.params = { app_id: 'trashtech' };
      req.body = {};

      const getAppIdFromAuth = jest.fn(() => 'trashtech');
      const middleware = requireAppId({ getAppIdFromAuth });

      middleware(req, res, next);

      expect(getAppIdFromAuth).toHaveBeenCalledWith(req);
      expect(req.verifiedAppId).toBe('trashtech');
      expect(next).toHaveBeenCalledWith();
    });

    it('should reject request when auth function returns different app_id', () => {
      req.params = { app_id: 'trashtech' };
      req.body = {};

      const getAppIdFromAuth = jest.fn(() => 'apping');
      const middleware = requireAppId({ getAppIdFromAuth });

      middleware(req, res, next);

      expect(res.status).toHaveBeenCalledWith(403);
      expect(res.json).toHaveBeenCalledWith({ error: 'Forbidden: Cannot access other app data' });
      expect(next).not.toHaveBeenCalled();
    });

    it('should reject request when auth returns null', () => {
      req.params = { app_id: 'trashtech' };
      req.body = {};

      const getAppIdFromAuth = jest.fn(() => null);
      const middleware = requireAppId({ getAppIdFromAuth });

      middleware(req, res, next);

      expect(res.status).toHaveBeenCalledWith(401);
      expect(res.json).toHaveBeenCalledWith({ error: 'Unauthorized: No app_id in token' });
    });

    it('should get app_id from query params', () => {
      req.params = {};
      req.body = {};
      req.query = { app_id: 'trashtech' };

      const middleware = requireAppId();
      middleware(req, res, next);

      expect(req.verifiedAppId).toBe('trashtech');
      expect(next).toHaveBeenCalledWith();
    });
  });

  describe('rejectSensitiveData', () => {
    it('should allow request with safe data', () => {
      req.body = {
        billing_customer_id: 1,
        payment_method_id: 'pm_test_123',
        plan_id: 'pro-monthly',
        price_cents: 9900
      };

      rejectSensitiveData(req, res, next);

      expect(next).toHaveBeenCalledWith();
      expect(res.status).not.toHaveBeenCalled();
    });

    it('should reject request with card_number', () => {
      req.body = {
        card_number: '4242424242424242',
        payment_method_id: 'pm_test_123'
      };

      rejectSensitiveData(req, res, next);

      expect(res.status).toHaveBeenCalledWith(400);
      expect(res.json).toHaveBeenCalledWith({
        error: 'PCI violation: Use Tilled hosted fields'
      });
      expect(next).not.toHaveBeenCalled();
    });

    it('should reject request with cvv', () => {
      req.body = {
        cvv: '123',
        payment_method_id: 'pm_test_123'
      };

      rejectSensitiveData(req, res, next);

      expect(res.status).toHaveBeenCalledWith(400);
      expect(res.json).toHaveBeenCalledWith({
        error: 'PCI violation: Use Tilled hosted fields'
      });
    });

    it('should reject request with account_number', () => {
      req.body = {
        account_number: '123456789',
        payment_method_id: 'pm_test_123'
      };

      rejectSensitiveData(req, res, next);

      expect(res.status).toHaveBeenCalledWith(400);
    });

    it('should detect sensitive fields case-insensitively', () => {
      req.body = {
        CARD_NUMBER: '4242424242424242'
      };

      rejectSensitiveData(req, res, next);

      expect(res.status).toHaveBeenCalledWith(400);
    });

    it('should detect sensitive fields in nested objects', () => {
      req.body = {
        payment: {
          card_number: '4242424242424242'
        }
      };

      rejectSensitiveData(req, res, next);

      expect(res.status).toHaveBeenCalledWith(400);
    });

    it('should allow partial matches in field names', () => {
      req.body = {
        subscription_id: 'sub_123',
        customer_account_id: 'cust_456'
      };

      rejectSensitiveData(req, res, next);

      expect(next).toHaveBeenCalledWith();
    });
  });
});
