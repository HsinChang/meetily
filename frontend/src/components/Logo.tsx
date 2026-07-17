import React from "react";
import Image from "next/image";

interface LogoProps {
    isCollapsed: boolean;
}

const Logo = ({ isCollapsed }: LogoProps) => {
  return isCollapsed ? (
    <div className="flex items-center justify-center mb-2">
      <Image src="/penplup.png" alt="Meetily" width={18} height={40} priority />
    </div>
  ) : (
    <span className="text-lg text-center border rounded-full bg-blue-50 border-white font-semibold text-gray-700 mb-2 block items-center">
      <span>Meetily</span>
    </span>
  );
};

Logo.displayName = "Logo";

export default Logo;
