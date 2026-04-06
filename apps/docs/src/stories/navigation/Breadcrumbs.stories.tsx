import type { Meta, StoryObj } from "@storybook/react";
import { Breadcrumbs } from "@7d/ui";

const meta: Meta<typeof Breadcrumbs> = {
  title: "Navigation/Breadcrumbs",
  component: Breadcrumbs,
  tags: ["autodocs"],
  parameters: { layout: "padded" },
};

export default meta;
type Story = StoryObj<typeof Breadcrumbs>;

export const Default: Story = {
  args: {
    items: [
      { label: "Home", href: "/" },
      { label: "Inventory", href: "/inventory" },
      { label: "Raw Materials" },
    ],
  },
};

export const Short: Story = {
  args: {
    items: [
      { label: "Dashboard", href: "/" },
      { label: "Settings" },
    ],
  },
};

export const WithCustomSeparator: Story = {
  args: {
    items: [
      { label: "Home", href: "/" },
      { label: "Reports", href: "/reports" },
      { label: "Q1 2026" },
    ],
    separator: "/",
  },
};
