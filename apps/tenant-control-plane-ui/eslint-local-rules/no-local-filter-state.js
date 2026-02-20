// ============================================================
// ESLint rule: no-local-filter-state
// Prevents useState for filter state.
// Use useFilterStore from @/infrastructure/state/useFilterStore instead.
// ============================================================
'use strict';

/** @type {import('eslint').Rule.RuleModule} */
module.exports = {
  meta: {
    type: 'suggestion',
    docs: {
      description: 'Disallow local useState for filter state; use useFilterStore instead.',
      category: 'Best Practices',
    },
    schema: [],
    messages: {
      noLocalFilterState:
        'Do not use local useState for filter state. ' +
        'Use useFilterStore from @/infrastructure/state/useFilterStore instead.',
    },
  },
  create(context) {
    const FILTER_PATTERNS = /^(filter|filters|activeFilter|selectedFilter|filterState)/i;

    return {
      VariableDeclarator(node) {
        if (
          node.init?.type === 'CallExpression' &&
          node.init.callee.name === 'useState' &&
          node.id?.type === 'ArrayPattern'
        ) {
          const [stateVar] = node.id.elements;
          if (stateVar?.type === 'Identifier' && FILTER_PATTERNS.test(stateVar.name)) {
            context.report({ node, messageId: 'noLocalFilterState' });
          }
        }
      },
    };
  },
};
