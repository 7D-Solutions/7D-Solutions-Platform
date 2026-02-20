// ============================================================
// ESLint rule: no-raw-button
// Prevents use of raw <button> elements in JSX.
// All buttons must use the Button component from @/components/ui
// which enforces double-click protection and consistent styling.
//
// Exception: buttons inside @/components/ui/ themselves are allowed.
// ============================================================
'use strict';

/** @type {import('eslint').Rule.RuleModule} */
module.exports = {
  meta: {
    type: 'suggestion',
    docs: {
      description: 'Disallow raw <button> elements; use Button from @/components/ui instead.',
      category: 'Best Practices',
    },
    schema: [],
    messages: {
      noRawButton:
        'Do not use raw <button> elements. ' +
        'Use Button from @/components/ui instead (enforces double-click protection).',
    },
  },
  create(context) {
    // Allow raw buttons inside the UI components directory itself
    const filename = context.getFilename();
    if (filename.includes('/components/ui/')) {
      return {};
    }

    return {
      JSXOpeningElement(node) {
        if (
          node.name.type === 'JSXIdentifier' &&
          node.name.name === 'button'
        ) {
          context.report({ node, messageId: 'noRawButton' });
        }
      },
    };
  },
};
