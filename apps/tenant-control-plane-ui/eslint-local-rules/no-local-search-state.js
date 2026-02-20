// ============================================================
// ESLint rule: no-local-search-state
// Prevents useState for search term state in feature code.
// Use useSearchStore from @/infrastructure/state/useSearchStore instead.
//
// Exclusions: components/ui/ and infrastructure/ may use local state
// legitimately (e.g. SearchableSelect's internal search input).
// ============================================================
'use strict';

/** @type {import('eslint').Rule.RuleModule} */
module.exports = {
  meta: {
    type: 'suggestion',
    docs: {
      description: 'Disallow local useState for search state in feature code; use useSearchStore instead.',
      category: 'Best Practices',
    },
    schema: [],
    messages: {
      noLocalSearchState:
        'Do not use local useState for search state. ' +
        'Use useSearchStore from @/infrastructure/state/useSearchStore instead.',
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

    // Match specific search term variable names; avoid false positives like 'queryClient'
    const SEARCH_PATTERNS = /^(searchTerm|searchQuery|searchText|searchInput|searchValue|searchString|search)$/i;

    return {
      VariableDeclarator(node) {
        if (
          node.init?.type === 'CallExpression' &&
          node.init.callee.name === 'useState' &&
          node.id?.type === 'ArrayPattern'
        ) {
          const [stateVar] = node.id.elements;
          if (stateVar?.type === 'Identifier' && SEARCH_PATTERNS.test(stateVar.name)) {
            context.report({ node, messageId: 'noLocalSearchState' });
          }
        }
      },
    };
  },
};
