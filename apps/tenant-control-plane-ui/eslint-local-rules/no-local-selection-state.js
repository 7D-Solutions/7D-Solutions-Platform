// ============================================================
// ESLint rule: no-local-selection-state
// Prevents useState for row/item selection state.
// Use useSelectionStore from @/infrastructure/state/useSelectionStore instead.
// ============================================================
'use strict';

/** @type {import('eslint').Rule.RuleModule} */
module.exports = {
  meta: {
    type: 'suggestion',
    docs: {
      description: 'Disallow local useState for selection state; use useSelectionStore instead.',
      category: 'Best Practices',
    },
    schema: [],
    messages: {
      noLocalSelectionState:
        'Do not use local useState for selection state. ' +
        'Use useSelectionStore from @/infrastructure/state/useSelectionStore instead.',
    },
  },
  create(context) {
    const SELECTION_PATTERNS = /^(selected|selection|selectedItems|selectedRows|checkedItems)/i;

    return {
      VariableDeclarator(node) {
        if (
          node.init?.type === 'CallExpression' &&
          node.init.callee.name === 'useState' &&
          node.id?.type === 'ArrayPattern'
        ) {
          const [stateVar] = node.id.elements;
          if (stateVar?.type === 'Identifier' && SELECTION_PATTERNS.test(stateVar.name)) {
            context.report({ node, messageId: 'noLocalSelectionState' });
          }
        }
      },
    };
  },
};
