import type { Meta, StoryObj } from "@storybook/react";
import { Pagination } from "@7d/ui";
import { useState } from "react";

const meta: Meta<typeof Pagination> = {
  title: "Navigation/Pagination",
  component: Pagination,
  tags: ["autodocs"],
  parameters: { layout: "padded" },
};

export default meta;
type Story = StoryObj<typeof Pagination>;

export const Default: Story = {
  render: () => {
    const [page, setPage] = useState(1);
    return <Pagination page={page} totalPages={10} onPageChange={setPage} />;
  },
};

export const ManyPages: Story = {
  render: () => {
    const [page, setPage] = useState(5);
    return <Pagination page={page} totalPages={50} onPageChange={setPage} />;
  },
};

export const SinglePage: Story = {
  render: () => <Pagination page={1} totalPages={1} onPageChange={() => {}} />,
};
