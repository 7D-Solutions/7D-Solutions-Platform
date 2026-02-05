const { validationResult, param, body, query } = require('express-validator');

/**
 * Middleware to handle validation errors
 * Returns 400 with array of error messages if validation fails
 */
const handleValidationErrors = (req, res, next) => {
  const errors = validationResult(req);
  if (!errors.isEmpty()) {
    return res.status(400).json({
      error: 'Validation failed',
      details: errors.array().map(err => ({
        field: err.path || err.param,
        message: err.msg
      }))
    });
  }
  next();
};

// ─── Composable Validator Factories ──────────────────────────────

/**
 * Required positive integer in URL param
 * @param {string} name - Parameter name
 * @param {string} [label] - Human-readable label for error messages
 */
const positiveIntParam = (name, label) => {
  const displayName = label || name.replace(/_/g, ' ');
  return param(name)
    .isInt({ min: 1 })
    .withMessage(`${displayName} must be a positive integer`);
};

/**
 * Positive integer in request body (with .toInt())
 * @param {string} name - Field name
 * @param {Object} [options] - Configuration options
 * @param {string} [options.label] - Human-readable label for error messages
 * @param {boolean} [options.required=true] - Whether field is required
 */
const positiveIntBody = (name, optionsOrLabel = {}) => {
  const options = typeof optionsOrLabel === 'string' ? { label: optionsOrLabel } : optionsOrLabel;
  const { label, required = true } = options;
  const displayName = label || name;
  const chain = body(name);
  if (required) {
    chain.notEmpty().withMessage(`${displayName} is required`);
  } else {
    chain.optional();
  }
  return chain
    .isInt({ min: 1 })
    .withMessage(`${displayName} must be a positive integer`)
    .toInt();
};

/**
 * Positive integer in query string (with .toInt())
 * @param {string} name - Field name
 * @param {Object} [options] - Configuration options
 * @param {string} [options.label] - Human-readable label for error messages
 * @param {boolean} [options.required=false] - Whether field is required
 */
const positiveIntQuery = (name, options = {}) => {
  const { label, required = false } = options;
  const displayName = label || name;
  const chain = query(name);
  if (required) {
    chain.notEmpty().withMessage(`${displayName} is required`);
  } else {
    chain.optional();
  }
  const message = required
    ? `${displayName} must be a positive integer`
    : `${displayName} must be a positive integer if provided`;
  return chain
    .isInt({ min: 1 })
    .withMessage(message)
    .toInt();
};

/**
 * ISO 8601 date field
 * @param {'body'|'query'} source - Where the field comes from
 * @param {string} name - Field name
 * @param {Object} [opts]
 * @param {boolean} [opts.required=false] - Whether the field is required
 * @param {string} [opts.label] - Human-readable label for error messages
 */
const isoDateField = (source, name, opts = {}) => {
  const { required = false, label } = opts;
  const displayName = label || name;
  const fn = source === 'body' ? body : query;
  let chain = fn(name);
  if (required) {
    chain = chain
      .notEmpty()
      .withMessage(`${displayName} is required`);
  } else {
    chain = chain.optional();
  }
  return chain
    .isISO8601()
    .withMessage(`${displayName} must be a valid ISO 8601 date`);
};

/**
 * Date range pair with cross-field validation (end > start)
 * @param {'body'|'query'} source - Where the fields come from
 * @param {string} startField - Start date field name
 * @param {string} endField - End date field name
 * @param {Object} [opts]
 * @param {boolean} [opts.required=false] - Whether both fields are required
 */
const dateRangeFields = (source, startField, endField, opts = {}) => {
  const { required = false } = opts;
  const startValidator = isoDateField(source, startField, { required });
  const fn = source === 'body' ? body : query;

  let endChain = fn(endField);
  if (required) {
    endChain = endChain
      .notEmpty()
      .withMessage(`${endField} is required`);
  } else {
    endChain = endChain.optional();
  }

  const endValidator = endChain
    .isISO8601()
    .withMessage(`${endField} must be a valid ISO 8601 date`)
    .custom((value, { req }) => {
      const startValue = source === 'body' ? req.body[startField] : req.query[startField];
      if (startValue && new Date(value) <= new Date(startValue)) {
        throw new Error(`${endField} must be after ${startField}`);
      }
      return true;
    });

  return [startValidator, endValidator];
};

/**
 * Enum field (isIn validator)
 * @param {'body'|'query'} source - Where the field comes from
 * @param {string} name - Field name
 * @param {string[]} values - Allowed values
 * @param {Object} [opts]
 * @param {boolean} [opts.required=false] - Whether the field is required
 * @param {string} [opts.label] - Human-readable label
 */
const enumField = (source, name, values, opts = {}) => {
  const { required = false, label } = opts;
  const displayName = label || name;
  const fn = source === 'body' ? body : query;
  let chain = fn(name);
  if (required) {
    chain = chain
      .notEmpty()
      .withMessage(`${displayName} is required`);
  } else {
    chain = chain.optional();
  }
  return chain
    .isIn(values)
    .withMessage(`${displayName} must be one of: ${values.join(', ')}`);
};

/**
 * Pagination query parameters (limit + offset)
 * @param {Object} [defaults]
 * @param {number} [defaults.maxLimit=100] - Maximum allowed limit
 */
const paginationQuery = (defaults = {}) => {
  const { maxLimit = 100 } = defaults;
  return [
    query('limit')
      .optional()
      .isInt({ min: 1, max: maxLimit })
      .withMessage(`limit must be between 1 and ${maxLimit}`)
      .toInt(),
    query('offset')
      .optional()
      .isInt({ min: 0 })
      .withMessage('offset must be a non-negative integer')
      .toInt()
  ];
};

/**
 * Non-negative integer amount in cents
 * @param {'body'|'query'} source - Where the field comes from
 * @param {string} name - Field name
 * @param {Object} [opts]
 * @param {boolean} [opts.required=true] - Whether the field is required
 * @param {number} [opts.min=0] - Minimum value (0 = non-negative, 1 = positive)
 * @param {string} [opts.label] - Human-readable label
 */
const amountCentsField = (source, name, opts = {}) => {
  const { required = true, min = 0, label } = opts;
  const displayName = label || name;
  const fn = source === 'body' ? body : query;
  let chain = fn(name);
  if (required) {
    chain = chain
      .notEmpty()
      .withMessage(`${displayName} is required`);
  } else {
    chain = chain.optional();
  }
  const minLabel = min === 0 ? 'a non-negative integer' : 'a positive integer';
  return chain
    .isInt({ min })
    .withMessage(`${displayName} must be ${minLabel}`)
    .toInt();
};

module.exports = {
  handleValidationErrors,
  positiveIntParam,
  positiveIntBody,
  positiveIntQuery,
  isoDateField,
  dateRangeFields,
  enumField,
  paginationQuery,
  amountCentsField
};
