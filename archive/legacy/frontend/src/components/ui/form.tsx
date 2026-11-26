import { zodResolver } from "@hookform/resolvers/zod";
import type { ReactNode } from "react";
import { useForm, type DefaultValues, type FieldValues, type Resolver, type UseFormReturn } from "react-hook-form";
import type { ZodType } from "zod";

export function useZodForm<T extends FieldValues>(schema: ZodType<T>, defaults: DefaultValues<T>) {
  return useForm<T>({
    resolver: zodResolver(schema as never) as Resolver<T>,
    defaultValues: defaults,
  });
}

export function Form({
  children,
  onSubmit,
  label,
}: {
  children: ReactNode;
  onSubmit: () => void;
  label: string;
}) {
  return (
    <form
      data-token="form"
      aria-label={label}
      className="flex flex-col gap-3"
      onSubmit={(event) => {
        event.preventDefault();
        onSubmit();
      }}
    >
      {children}
    </form>
  );
}

export type FormApi<T extends FieldValues> = UseFormReturn<T>;
