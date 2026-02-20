// ============================================================
// TCP UI — local ESLint rules registry
// All 9 custom rules exported for use in eslint.config.js.
// ============================================================
'use strict';

module.exports = {
  rules: {
    'no-local-modal-state':      require('./no-local-modal-state'),
    'no-local-form-state':       require('./no-local-form-state'),
    'no-local-filter-state':     require('./no-local-filter-state'),
    'no-local-search-state':     require('./no-local-search-state'),
    'no-local-upload-state':     require('./no-local-upload-state'),
    'no-local-selection-state':  require('./no-local-selection-state'),
    'no-local-view-state':       require('./no-local-view-state'),
    'no-raw-button':             require('./no-raw-button'),
    'no-browser-notifications':  require('./no-browser-notifications'),
  },
};
