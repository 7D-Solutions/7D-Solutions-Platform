module.exports = {
  coverageDirectory: 'coverage',
  collectCoverageFrom: [
    'backend/src/**/*.js',
    '!backend/src/**/*.test.js'
  ],
  verbose: true,

  // When using `projects`, root-level testMatch/setupFiles must be removed.
  // Otherwise Jest creates an implicit 4th project that duplicates every test
  // without the project-specific config (resetModules, setupFilesAfterEnv, etc.),
  // causing cross-suite contamination in integration tests.
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
      resetModules: false,  // Shared module cache — keeps single Prisma client across files
      clearMocks: true,
      // IMPORTANT: Integration tests MUST run with --runInBand (see npm test:integration).
      // maxWorkers:1 alone is insufficient — Jest still batches setupFilesAfterEnv
      // beforeAll hooks across files, causing cross-suite DB contamination.
      maxWorkers: 1
    },
    {
      displayName: 'real',
      testEnvironment: 'node',
      testMatch: ['<rootDir>/tests/real/**/*.test.js'],
      setupFiles: ['<rootDir>/tests/setup.js']
    }
  ]
};
