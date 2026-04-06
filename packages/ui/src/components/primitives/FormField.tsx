import React, { useId } from "react";
import { cn } from "../../lib/cn.js";
import { Label } from "./Label.js";
import { HelperText } from "./HelperText.js";

export interface FormFieldProps {
  label?: string;
  required?: boolean;
  /** Error message — also sets error state on the child */
  error?: string;
  /** Hint shown below the field when there is no error */
  hint?: string;
  /** Explicit id — auto-generated when omitted */
  id?: string;
  className?: string;
  children: (props: {
    id: string;
    describedBy: string | undefined;
    error: boolean;
  }) => React.ReactNode;
}

export function FormField({
  label,
  required,
  error,
  hint,
  id: idProp,
  className,
  children,
}: FormFieldProps) {
  const autoId = useId();
  const id = idProp ?? autoId;
  const helperId = `${id}-helper`;

  const hasHelper = Boolean(error ?? hint);

  return (
    <div className={cn("flex flex-col gap-1", className)}>
      {label && (
        <Label htmlFor={id} required={required}>
          {label}
        </Label>
      )}
      {children({
        id,
        describedBy: hasHelper ? helperId : undefined,
        error: Boolean(error),
      })}
      {hasHelper && (
        <HelperText id={helperId} error={Boolean(error)}>
          {error ?? hint}
        </HelperText>
      )}
    </div>
  );
}
