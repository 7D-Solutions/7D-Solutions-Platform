import type { Meta, StoryObj } from "@storybook/react";
import { FormField, Input } from "@7d/ui";

const meta: Meta<typeof FormField> = {
  title: "Primitives/FormField",
  component: FormField,
  tags: ["autodocs"],
};

export default meta;
type Story = StoryObj<typeof FormField>;

export const Default: Story = {
  render: () => (
    <FormField label="Email address" hint="We will never share your email.">
      {({ id, describedBy, error }) => (
        <Input id={id} describedBy={describedBy} error={error} placeholder="you@example.com" />
      )}
    </FormField>
  ),
};

export const Required: Story = {
  render: () => (
    <FormField label="Username" required>
      {({ id, describedBy, error }) => (
        <Input id={id} describedBy={describedBy} error={error} placeholder="username" />
      )}
    </FormField>
  ),
};

export const WithError: Story = {
  render: () => (
    <FormField label="Password" error="Password must be at least 8 characters.">
      {({ id, describedBy, error }) => (
        <Input id={id} describedBy={describedBy} error={error} type="password" />
      )}
    </FormField>
  ),
};
