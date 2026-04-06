import type { Meta, StoryObj } from "@storybook/react";
import { Skeleton } from "@7d/ui";

const meta: Meta<typeof Skeleton> = {
  title: "Primitives/Skeleton",
  component: Skeleton,
  tags: ["autodocs"],
};

export default meta;
type Story = StoryObj<typeof Skeleton>;

export const Default: Story = {
  args: { width: 200, height: 20 },
};

export const Circle: Story = {
  args: { width: 40, height: 40, circle: true },
};

export const CardPlaceholder: Story = {
  render: () => (
    <div className="flex flex-col gap-2 w-64">
      <Skeleton height={160} />
      <Skeleton height={16} width="75%" />
      <Skeleton height={14} width="50%" />
    </div>
  ),
};
