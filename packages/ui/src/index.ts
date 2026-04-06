// Class name utility
export { cn } from "./lib/cn.js";

// Accessibility helpers
export {
  srOnly,
  ariaId,
  ariaInvalid,
  ariaDescribedBy,
} from "./lib/a11y.js";

// Focus management
export {
  getFocusable,
  getFocusBoundaries,
  trapFocus,
  moveFocus,
} from "./lib/focus.js";

// Keyboard utilities
export { Keys, isKey, onKey, isActivationKey } from "./lib/keyboard.js";
export type { Key } from "./lib/keyboard.js";

// Primitives — form + feedback
export {
  Button,
  Input,
  Textarea,
  Checkbox,
  RadioGroup,
  Switch,
  Label,
  FormField,
  HelperText,
  Spinner,
  Skeleton,
  Separator,
  Tooltip,
  Badge,
} from "./components/primitives/index.js";
export type {
  ButtonProps,
  ButtonVariant,
  ButtonSize,
  InputProps,
  InputSize,
  TextareaProps,
  CheckboxProps,
  RadioGroupProps,
  RadioOption,
  SwitchProps,
  LabelProps,
  FormFieldProps,
  HelperTextProps,
  SpinnerProps,
  SpinnerSize,
  SkeletonProps,
  SeparatorProps,
  TooltipProps,
  TooltipPlacement,
  BadgeProps,
  BadgeVariant,
  BadgeSize,
} from "./components/primitives/index.js";
