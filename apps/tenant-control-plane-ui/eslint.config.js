// ============================================================
// ESLint flat config (ESLint 9)
// Wires in Next.js recommended rules + all 9 local custom rules.
// ============================================================
'use strict';
const path = require('path');
const { FlatCompat } = require('@eslint/eslintrc');
const localRules = require('./eslint-local-rules/index.js');

const compat = new FlatCompat({ baseDirectory: __dirname });

/** @type {import('eslint').Linter.Config[]} */
const config = [
  // Next.js recommended config (TypeScript + React)
  ...compat.extends('next/core-web-vitals', 'next/typescript'),

  // Register all local custom rules
  {
    plugins: {
      local: localRules,
    },
    rules: {
      'local/no-local-modal-state':     'error',
      'local/no-local-form-state':      'error',
      'local/no-local-filter-state':    'error',
      'local/no-local-search-state':    'error',
      'local/no-local-upload-state':    'error',
      'local/no-local-selection-state': 'error',
      'local/no-local-view-state':      'error',
      'local/no-raw-button':            'error',
      'local/no-browser-notifications': 'error',
    },
  },

  // Ignore build artifacts and config files
  {
    ignores: [
      '.next/**',
      'node_modules/**',
      'eslint-local-rules/**',
      'playwright.config.ts',
      'postcss.config.js',
      'tailwind.config.ts',
    ],
  },
];

module.exports = config;
