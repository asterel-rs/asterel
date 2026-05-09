import { useId } from "react";
import { cn } from "@/lib/cn";
import { useI18n } from "@/lib/i18n";

interface FormInputProps {
  label: string;
  value: string;
  onChange: (value: string) => void;
  placeholder: string;
  type?: string;
  error?: string;
  className?: string;
}

export function FormInput({
  label,
  value,
  onChange,
  placeholder,
  type = "text",
  error,
  className,
}: FormInputProps) {
  const id = useId();
  const { t } = useI18n();
  const errorId = error ? `${id}-error` : undefined;
  return (
    <div className={className}>
      <label htmlFor={id} className="ui-field-label">
        {t(label)}
      </label>
      <input
        id={id}
        type={type}
        value={value}
        onChange={(e) => onChange(e.target.value)}
        placeholder={t(placeholder)}
        className={cn("ui-field", error && "ui-field-error")}
        aria-invalid={!!error}
        aria-describedby={errorId}
      />
      {error && (
        <p id={errorId} className="mt-1 text-xs text-[var(--error)]">
          {t(error)}
        </p>
      )}
    </div>
  );
}

interface FormTextareaProps {
  label: string;
  value: string;
  onChange: (value: string) => void;
  placeholder: string;
  rows?: number;
  error?: string;
  className?: string;
}

export function FormTextarea({
  label,
  value,
  onChange,
  placeholder,
  rows = 3,
  error,
  className,
}: FormTextareaProps) {
  const id = useId();
  const { t } = useI18n();
  const errorId = error ? `${id}-error` : undefined;
  return (
    <div className={className}>
      <label htmlFor={id} className="ui-field-label">
        {t(label)}
      </label>
      <textarea
        id={id}
        value={value}
        onChange={(e) => onChange(e.target.value)}
        placeholder={t(placeholder)}
        rows={rows}
        className={cn("ui-field resize-none", error && "ui-field-error")}
        aria-invalid={!!error}
        aria-describedby={errorId}
      />
      {error && (
        <p id={errorId} className="mt-1 text-xs text-[var(--error)]">
          {t(error)}
        </p>
      )}
    </div>
  );
}
