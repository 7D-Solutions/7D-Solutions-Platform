// ============================================================
// ESLint rule: no-local-modal-state
// Prevents useState for modal open/close state.
// Use useTabModal / useModalStore instead.
// ============================================================
'use strict';

/** @type {import('eslint').Rule.RuleModule} */
module.exports = {
  meta: {
    type: 'suggestion',
    docs: {
      description: 'Disallow local useState for modal visibility; use useTabModal instead.',
      category: 'Best Practices',
    },
    schema: [],
    messages: {
      noLocalModalState:
        'Do not use local useState for modal state. ' +
        'Use useTabModal from @/infrastructure/state/useTabModal instead.',
    },
  },
  create(context) {
    const MODAL_PATTERNS = /^(show|open|is|visible)(Modal|Dialog|Drawer|Sheet|Popup)/i;

    return {
      VariableDeclarator(node) {
        if (
          node.init?.type === 'CallExpression' &&
          node.init.callee.name === 'useState' &&
          node.id?.type === 'ArrayPattern'
        ) {
          const [stateVar] = node.id.elements;
          if (stateVar?.type === 'Identifier' && MODAL_PATTERNS.test(stateVar.name)) {
            context.report({ node, messageId: 'noLocalModalState' });
          }
        }
      },
    };
  },
};
