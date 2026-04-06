// DataTable
export {
  DataTable,
  DataTableToolbar,
  ColumnManager,
  SelectAllCheckbox,
  RowCheckbox,
} from "./components/data-table/index.js";
export type {
  DataTableProps,
  ColumnDef,
  SortState,
  SortDirection,
  DataTableToolbarProps,
  ColumnManagerProps,
} from "./components/data-table/index.js";

// Forms
export { SearchableSelect, FileUpload } from "./components/forms/index.js";
export type {
  SearchableSelectProps,
  SelectOption,
  SearchableSelectSize,
  FileUploadProps,
} from "./components/forms/index.js";

// Overlays
export {
  Modal,
  Drawer,
  Toast,
  ToastContainer,
} from "./components/overlays/index.js";
export type {
  ModalProps,
  ModalSize,
  DrawerProps,
  DrawerSide,
  DrawerSize,
  ToastProps,
  ToastVariant,
  ToastContainerProps,
  ToastPosition,
} from "./components/overlays/index.js";

// Navigation
export { Breadcrumbs, Pagination } from "./components/navigation/index.js";
export type {
  BreadcrumbsProps,
  BreadcrumbItem,
  PaginationProps,
} from "./components/navigation/index.js";

// Hooks
export {
  useLoadingState,
  useSearchDebounce,
  useBeforeUnload,
  usePagination,
  useColumnManager,
  useMutationPattern,
  useQueryInvalidation,
  registerQueryClient,
} from "./hooks/index.js";
export type {
  LoadingState,
  SearchDebounce,
  PaginationResult,
  Column,
  ColumnManagerResult,
  MutationConfig,
  MutationResult,
  QueryClientLike,
  QueryInvalidationResult,
} from "./hooks/index.js";

// Stores
export {
  modalStore,
  notificationStore,
  selectionStore,
  uploadStore,
} from "./stores/index.js";
export type {
  ModalEntry,
  Notification,
  NotificationVariant,
  FileMetadata,
  UploadStatus,
} from "./stores/index.js";

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

// Primitives — form + feedback + layout
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
  SkeletonText,
  SkeletonCard,
  SkeletonRow,
  SkeletonTable,
  SkeletonStat,
  Separator,
  Tooltip,
  Badge,
  EmptyState,
  EmptyStateInline,
  GlassCard,
  GlassCardHeader,
  GlassCardTitle,
  GlassCardDescription,
  GlassCardContent,
  GlassCardFooter,
  PageHeader,
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
  EmptyStateProps,
  GlassCardProps,
  GlassCardVariant,
  GlassCardPadding,
  PageHeaderProps,
} from "./components/primitives/index.js";
