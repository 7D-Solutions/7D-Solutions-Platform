// ============================================================
// ESLint rule: no-browser-notifications
// Prevents direct use of the browser Notification API.
// Use notificationStore from @/infrastructure/state/notificationStore instead.
// ============================================================
'use strict';

/** @type {import('eslint').Rule.RuleModule} */
module.exports = {
  meta: {
    type: 'suggestion',
    docs: {
      description:
        'Disallow direct browser Notification API; use notificationStore instead.',
      category: 'Best Practices',
    },
    schema: [],
    messages: {
      noBrowserNotifications:
        'Do not use the browser Notification API directly. ' +
        'Use notificationStore from @/infrastructure/state/notificationStore instead.',
    },
  },
  create(context) {
    return {
      MemberExpression(node) {
        if (
          node.object.type === 'Identifier' &&
          node.object.name === 'Notification' &&
          node.property.type === 'Identifier' &&
          node.property.name === 'requestPermission'
        ) {
          context.report({ node, messageId: 'noBrowserNotifications' });
        }
      },
      NewExpression(node) {
        if (
          node.callee.type === 'Identifier' &&
          node.callee.name === 'Notification'
        ) {
          context.report({ node, messageId: 'noBrowserNotifications' });
        }
      },
    };
  },
};
