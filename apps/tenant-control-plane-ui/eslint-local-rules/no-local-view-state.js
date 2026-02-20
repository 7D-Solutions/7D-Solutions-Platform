// ============================================================
// ESLint rule: no-local-view-state
// Prevents useState for view mode (list/card/grid) state.
// Use ViewToggle component (backed by userPreferencesService) instead.
// ============================================================
'use strict';

/** @type {import('eslint').Rule.RuleModule} */
module.exports = {
  meta: {
    type: 'suggestion',
    docs: {
      description: 'Disallow local useState for view mode; use ViewToggle component instead.',
      category: 'Best Practices',
    },
    schema: [],
    messages: {
      noLocalViewState:
        'Do not use local useState for view mode state. ' +
        'Use the ViewToggle component from @/components/ui/ViewToggle instead.',
    },
  },
  create(context) {
    const VIEW_PATTERNS = /^(view|viewMode|displayMode|layout|layoutMode)/i;

    return {
      VariableDeclarator(node) {
        if (
          node.init?.type === 'CallExpression' &&
          node.init.callee.name === 'useState' &&
          node.id?.type === 'ArrayPattern'
        ) {
          const [stateVar] = node.id.elements;
          if (stateVar?.type === 'Identifier' && VIEW_PATTERNS.test(stateVar.name)) {
            context.report({ node, messageId: 'noLocalViewState' });
          }
        }
      },
    };
  },
};
