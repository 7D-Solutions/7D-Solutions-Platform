import type { Meta, StoryObj } from "@storybook/react";
import { SearchableSelect } from "@7d/ui";
import { useState } from "react";

const OPTIONS = [
  { value: "us", label: "United States" },
  { value: "ca", label: "Canada" },
  { value: "gb", label: "United Kingdom" },
  { value: "de", label: "Germany" },
  { value: "fr", label: "France" },
  { value: "au", label: "Australia" },
  { value: "jp", label: "Japan" },
];

const meta: Meta<typeof SearchableSelect> = {
  title: "Forms/SearchableSelect",
  component: SearchableSelect,
  tags: ["autodocs"],
};

export default meta;
type Story = StoryObj<typeof SearchableSelect>;

export const Default: Story = {
  render: () => {
    const [value, setValue] = useState<string | undefined>();
    return (
      <div className="w-64">
        <SearchableSelect
          options={OPTIONS}
          value={value}
          onChange={setValue}
          placeholder="Select a country…"
          aria-label="Country"
        />
      </div>
    );
  },
};

export const Clearable: Story = {
  render: () => {
    const [value, setValue] = useState<string>("us");
    return (
      <div className="w-64">
        <SearchableSelect
          options={OPTIONS}
          value={value}
          onChange={setValue}
          clearable
          aria-label="Country"
        />
      </div>
    );
  },
};

export const WithError: Story = {
  render: () => (
    <div className="w-64">
      <SearchableSelect
        options={OPTIONS}
        error
        placeholder="Required field"
        aria-label="Country"
      />
    </div>
  ),
};

export const Disabled: Story = {
  render: () => (
    <div className="w-64">
      <SearchableSelect
        options={OPTIONS}
        value="gb"
        disabled
        aria-label="Country"
      />
    </div>
  ),
};
