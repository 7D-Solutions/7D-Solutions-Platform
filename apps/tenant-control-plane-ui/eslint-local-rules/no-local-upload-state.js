// ============================================================
// ESLint rule: no-local-upload-state
// Prevents useState for file upload state in feature code.
// Use useUploadStore from @/infrastructure/state/useUploadStore instead.
//
// Exclusions: components/ui/ and infrastructure/ may use local state
// legitimately (e.g. FileInput component's internal file list).
// ============================================================
'use strict';

/** @type {import('eslint').Rule.RuleModule} */
module.exports = {
  meta: {
    type: 'suggestion',
    docs: {
      description: 'Disallow local useState for upload state in feature code; use useUploadStore instead.',
      category: 'Best Practices',
    },
    schema: [],
    messages: {
      noLocalUploadState:
        'Do not use local useState for upload state. ' +
        'Use useUploadStore from @/infrastructure/state/useUploadStore instead.',
    },
  },
  create(context) {
    const filename = context.getFilename();
    // Allow internal state in UI primitives and infrastructure
    if (
      filename.includes('/components/ui/') ||
      filename.includes('/infrastructure/')
    ) {
      return {};
    }

    const UPLOAD_PATTERNS = /^(uploads?|uploadedFile|selectedFile|uploadFiles?)$/i;

    return {
      VariableDeclarator(node) {
        if (
          node.init?.type === 'CallExpression' &&
          node.init.callee.name === 'useState' &&
          node.id?.type === 'ArrayPattern'
        ) {
          const [stateVar] = node.id.elements;
          if (stateVar?.type === 'Identifier' && UPLOAD_PATTERNS.test(stateVar.name)) {
            context.report({ node, messageId: 'noLocalUploadState' });
          }
        }
      },
    };
  },
};
