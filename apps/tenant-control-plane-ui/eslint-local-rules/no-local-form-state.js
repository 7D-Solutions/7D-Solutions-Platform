// ============================================================
// ESLint rule: no-local-form-state
// Prevents useState for form field tracking.
// Use useFormStore from @/infrastructure/state/useFormStore instead.
// ============================================================
'use strict';

/** @type {import('eslint').Rule.RuleModule} */
module.exports = {
  meta: {
    type: 'suggestion',
    docs: {
      description: 'Disallow local useState for form field state; use useFormStore instead.',
      category: 'Best Practices',
    },
    schema: [],
    messages: {
      noLocalFormState:
        'Do not use local useState for form field state. ' +
        'Use useFormStore from @/infrastructure/state/useFormStore instead.',
    },
  },
  create(context) {
    const FORM_PATTERNS = /^(form|field|input|value|values|formData|formValues|formState)/i;

    return {
      VariableDeclarator(node) {
        if (
          node.init?.type === 'CallExpression' &&
          node.init.callee.name === 'useState' &&
          node.id?.type === 'ArrayPattern'
        ) {
          const [stateVar] = node.id.elements;
          if (stateVar?.type === 'Identifier' && FORM_PATTERNS.test(stateVar.name)) {
            context.report({ node, messageId: 'noLocalFormState' });
          }
        }
      },
    };
  },
};
