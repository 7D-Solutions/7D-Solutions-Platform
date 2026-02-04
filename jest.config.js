module.exports = {
  testEnvironment: 'node',
  coverageDirectory: 'coverage',
  collectCoverageFrom: [
    'backend/src/**/*.js',
    '!backend/src/**/*.test.js'
  ],
  testMatch: [
    '**/__tests__/**/*.js',
    '**/*.test.js'
  ],
  setupFiles: ['<rootDir>/tests/setup.js'],
  verbose: true,

  // Test categorization using test path patterns
  projects: [
    {
      displayName: 'unit',
      testEnvironment: 'node',
      testMatch: ['<rootDir>/tests/unit/**/*.test.js'],
      setupFiles: ['<rootDir>/tests/setup.js']
    },
    {
      displayName: 'integration',
      testEnvironment: 'node',
      testMatch: ['<rootDir>/tests/integration/**/*.test.js'],
      setupFiles: ['<rootDir>/tests/setup.js'],
      setupFilesAfterEnv: ['<rootDir>/tests/integrationSetup.js'],
      resetModules: false,  // Don't reset modules to preserve Prisma client singleton
      clearMocks: true,
      maxWorkers: 1  // Run integration tests serially to avoid DB race conditions
    },
    {
      displayName: 'real',
      testEnvironment: 'node',
      testMatch: ['<rootDir>/tests/real/**/*.test.js'],
      setupFiles: ['<rootDir>/tests/setup.js']
    }
  ]
};
