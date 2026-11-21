class IsPalindrome {
    boolean isPalindrome(String text) {
        String cleaned = text.toLowerCase().replaceAll("[^a-z0-9]", "");
        int left = 0;
        int right = cleaned.length() - 1;
        boolean result = true;
        while (left < right) {
            if (cleaned.charAt(left) != cleaned.charAt(right)) {
                result = false;
                break;
            }
            left++;
            right--;
        }
        return result;
    }
}
