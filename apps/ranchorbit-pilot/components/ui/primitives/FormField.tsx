import React, { useId } from "react";
import { cn } from "../lib/cn";
import { Label } from "./Label";
import { HelperText } from "./HelperText";

export interface FormFieldProps {
  label?: string;
  required?: boolean;
  error?: string;
  hint?: string;
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
